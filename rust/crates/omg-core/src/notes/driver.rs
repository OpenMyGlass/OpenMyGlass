//! Binds each state machine to the store: runs its effects in order and hands back the
//! frames to send.
//!
//! Synchronous, and with no idea what carries the frames: a phone app that owns its own
//! Bluetooth or BLE socket calls [`GlassNotes`] or [`CompanionNotes`] directly and writes
//! the returned frames itself. Returned frames are safe to send: everything they claim is
//! already durable.
//!
//! An error means the store failed mid-way. Nothing from that call may be sent, and the
//! driver has reloaded itself from what the disk really holds, so its memory is never ahead
//! of its files. Treat it as a lost link: close the link and call `link_up` on a new one.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

use omg_protocol::{AttachmentId, AttachmentKind, Frame, Locale, NoteId};
use tracing::warn;

use super::{
    receiver::{NoteReceiver, ReceiverEffect, ReceiverEvent},
    record::{IncomingNote, OutgoingNote},
    sender::{NoteSender, SenderConfig, SenderEffect, SenderEvent},
    speech::{
        SpeechError, SpeechToText, SynthesizeRequest, SynthesizedAudio, TextToSpeech,
        TranscribeRequest,
    },
    store::NoteStore,
    LinkProfile, RetryPolicy,
};
use crate::CoreError;

/// One file to attach. It is copied into the queue; the original is left alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentSource {
    /// What it is.
    pub kind: AttachmentKind,
    /// MIME type.
    pub mime: String,
    /// Where it is now.
    pub path: PathBuf,
    /// Length of audio or video, when known.
    pub duration_ms: Option<u32>,
}

/// A note about to be sealed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteDraft {
    /// Glass wall clock, Unix ms.
    pub created_at_ms: u64,
    /// Language of the note.
    pub locale: Locale,
    /// Text, if any.
    pub text: Option<String>,
    /// Files, in the order they get their attachment ids.
    pub attachments: Vec<AttachmentSource>,
}

/// Seals one note, on any thread. See [`GlassNotes::sealer`].
pub struct NoteSealer {
    store: NoteStore,
    seq: u64,
}

impl NoteSealer {
    /// Copy and hash the files and seal the note atomically. Blocking.
    pub fn seal(self, draft: &NoteDraft) -> Result<OutgoingNote, CoreError> {
        let mut id = [0u8; 16];
        getrandom::getrandom(&mut id)
            .map_err(|e| CoreError::Store(format!("no randomness for a note id: {e}")))?;
        self.store.seal(NoteId(id), self.seq, draft)
    }
}

/// The Glass side.
pub struct GlassNotes {
    store: NoteStore,
    cfg: SenderConfig,
    machine: NoteSender,
    next_sealer_seq: u64,
    events: Vec<SenderEvent>,
    set_aside: Vec<String>,
}

impl GlassNotes {
    /// Open the queue at `root`, finishing what a crash interrupted.
    pub fn open(root: impl AsRef<Path>, cfg: SenderConfig) -> Result<GlassNotes, CoreError> {
        let store = NoteStore::open(root)?;
        let loaded = store.load::<OutgoingNote>()?;
        let mut notes = GlassNotes {
            store,
            cfg,
            machine: NoteSender::new(cfg, loaded.records),
            next_sealer_seq: 0,
            events: Vec::new(),
            set_aside: loaded.set_aside,
        };
        let effects = notes.machine.startup();
        notes.apply(effects, 0)?;
        Ok(notes)
    }

    /// Seal a note into the queue. It stays on the Glass until a link comes up, so this
    /// works the same with or without a companion. The frames are for a link already up.
    pub fn add_note(
        &mut self,
        draft: &NoteDraft,
        now: u64,
    ) -> Result<(NoteId, Vec<Frame>), CoreError> {
        let record = self.sealer().seal(draft)?;
        let id = record.manifest.id;
        let frames = self.enqueue_sealed(record, now)?;
        Ok((id, frames))
    }

    /// Start sealing a note on another thread: copying and hashing a video takes far too long
    /// for a thread that also services input. Run [`NoteSealer::seal`] there, then hand the
    /// record to [`enqueue_sealed`](GlassNotes::enqueue_sealed). Seal one at a time per
    /// sealer: the queue position is taken here.
    pub fn sealer(&mut self) -> NoteSealer {
        let seq = self.next_sealer_seq.max(self.machine.next_seq());
        self.next_sealer_seq = seq + 1;
        NoteSealer {
            store: self.store.clone(),
            seq,
        }
    }

    /// Queue a note a [`NoteSealer`] finished. Like [`add_note`](GlassNotes::add_note), the
    /// frames are for a link already up.
    pub fn enqueue_sealed(
        &mut self,
        record: OutgoingNote,
        now: u64,
    ) -> Result<Vec<Frame>, CoreError> {
        let effects = self.machine.enqueue(record, now);
        self.apply(effects, now)
    }

    /// See [`NoteSender::link_up`].
    pub fn link_up(&mut self, profile: LinkProfile, now: u64) -> Result<Vec<Frame>, CoreError> {
        let effects = self.machine.link_up(profile, now);
        self.apply(effects, now)
    }

    /// See [`NoteSender::link_down`].
    pub fn link_down(&mut self) {
        self.machine.link_down();
    }

    /// See [`NoteSender::on_frame`].
    pub fn on_frame(&mut self, frame: Frame, now: u64) -> Result<Vec<Frame>, CoreError> {
        let effects = self.machine.on_frame(frame, now);
        self.apply(effects, now)
    }

    /// See [`NoteSender::tick`].
    pub fn tick(&mut self, now: u64) -> Result<Vec<Frame>, CoreError> {
        let effects = self.machine.tick(now);
        self.apply(effects, now)
    }

    /// See [`NoteSender::retry_blocked`].
    pub fn retry_blocked(&mut self, id: &NoteId, now: u64) -> Result<Vec<Frame>, CoreError> {
        let effects = self.machine.retry_blocked(id, now);
        self.apply(effects, now)
    }

    /// See [`NoteSender::request_tts`]. `None` without a link.
    pub fn request_tts(
        &mut self,
        note_id: NoteId,
        text: Option<String>,
        locale: Locale,
        now: u64,
    ) -> Result<Option<(u32, Vec<Frame>)>, CoreError> {
        match self.machine.request_tts(note_id, text, locale, now) {
            Some((request_id, effects)) => Ok(Some((request_id, self.apply(effects, now)?))),
            None => Ok(None),
        }
    }

    /// Where a sealed attachment is.
    pub fn attachment_path(&self, id: &NoteId, attachment: AttachmentId) -> PathBuf {
        self.store.attachment_path(id, attachment)
    }

    /// The state machine, to read the queue from.
    pub fn machine(&self) -> &NoteSender {
        &self.machine
    }

    /// Events since the last call.
    pub fn take_events(&mut self) -> Vec<SenderEvent> {
        std::mem::take(&mut self.events)
    }

    /// Notes whose records could not be read at open, kept aside on disk.
    pub fn set_aside(&self) -> &[String] {
        &self.set_aside
    }

    fn apply(&mut self, effects: Vec<SenderEffect>, now: u64) -> Result<Vec<Frame>, CoreError> {
        let result = self.run_effects(effects, now);
        if result.is_err() {
            self.machine = NoteSender::new(self.cfg, self.store.load::<OutgoingNote>()?.records);
        }
        result
    }

    fn run_effects(
        &mut self,
        effects: Vec<SenderEffect>,
        now: u64,
    ) -> Result<Vec<Frame>, CoreError> {
        let mut queue: VecDeque<_> = effects.into();
        let mut frames = Vec::new();
        while let Some(effect) = queue.pop_front() {
            let more = match effect {
                SenderEffect::Send(frame) => {
                    frames.push(frame);
                    continue;
                }
                SenderEffect::Event(event) => {
                    self.events.push(event);
                    continue;
                }
                SenderEffect::Persist(record) => {
                    self.store.save(record.as_ref())?;
                    continue;
                }
                SenderEffect::DeleteLocal { note_id } => {
                    self.store.delete_note(&note_id)?;
                    continue;
                }
                SenderEffect::ReadChunk {
                    note_id,
                    attachment_id,
                    offset,
                    len,
                } => {
                    let data = self
                        .store
                        .read_chunk(&note_id, attachment_id, offset, len)
                        .map_err(|e| warn!(error = %e, "cannot read an attachment"));
                    self.machine
                        .on_chunk_read(note_id, attachment_id, offset, data)
                }
                SenderEffect::VerifySource {
                    note_id,
                    attachment_id,
                } => {
                    let actual = self.store.hash_attachment(&note_id, attachment_id).ok();
                    self.machine
                        .on_source_checked(note_id, attachment_id, actual, now)
                }
            };
            for effect in more.into_iter().rev() {
                queue.push_front(effect);
            }
        }
        Ok(frames)
    }
}

/// Speech work to run away from the link: it can take far longer than a heartbeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeechJob {
    /// Run speech-to-text.
    Transcribe(TranscribeRequest),
    /// Run TTS.
    Synthesize {
        /// The Glass's request id.
        request_id: u32,
        /// What to do.
        request: SynthesizeRequest,
    },
}

/// A finished [`SpeechJob`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeechOutcome {
    /// Speech-to-text finished.
    Transcribed {
        /// The note.
        note_id: NoteId,
        /// The voice attachment.
        attachment_id: AttachmentId,
        /// The text.
        result: Result<String, SpeechError>,
    },
    /// TTS finished.
    Synthesized {
        /// The Glass's request id.
        request_id: u32,
        /// The note.
        note_id: NoteId,
        /// The audio.
        result: Result<SynthesizedAudio, SpeechError>,
    },
}

impl SpeechJob {
    /// Run it. Blocking.
    pub fn run(&self, stt: &dyn SpeechToText, tts: &dyn TextToSpeech) -> SpeechOutcome {
        match self {
            SpeechJob::Transcribe(request) => SpeechOutcome::Transcribed {
                note_id: request.note_id,
                attachment_id: request.attachment_id,
                result: stt.transcribe(request),
            },
            SpeechJob::Synthesize {
                request_id,
                request,
            } => SpeechOutcome::Synthesized {
                request_id: *request_id,
                note_id: request.note_id,
                result: tts.synthesize(request),
            },
        }
    }

    /// The outcome for an engine that blew up instead of returning.
    pub fn failed(&self, why: &str) -> SpeechOutcome {
        let result_err = || SpeechError::Failed(why.to_string());
        match self {
            SpeechJob::Transcribe(request) => SpeechOutcome::Transcribed {
                note_id: request.note_id,
                attachment_id: request.attachment_id,
                result: Err(result_err()),
            },
            SpeechJob::Synthesize {
                request_id,
                request,
            } => SpeechOutcome::Synthesized {
                request_id: *request_id,
                note_id: request.note_id,
                result: Err(result_err()),
            },
        }
    }
}

/// The companion side.
pub struct CompanionNotes {
    store: NoteStore,
    retry: RetryPolicy,
    free_bytes: Option<u64>,
    machine: NoteReceiver,
    events: Vec<ReceiverEvent>,
    jobs: Vec<SpeechJob>,
    set_aside: Vec<String>,
}

impl CompanionNotes {
    /// Open the note store at `root` and reconcile it with what a crash may have left.
    pub fn open(root: impl AsRef<Path>, retry: RetryPolicy) -> Result<CompanionNotes, CoreError> {
        let store = NoteStore::open(root)?;
        let (records, set_aside) = load_reconciled(&store)?;
        Ok(CompanionNotes {
            store,
            retry,
            free_bytes: None,
            machine: NoteReceiver::new(retry, records),
            events: Vec::new(),
            jobs: Vec::new(),
            set_aside,
        })
    }

    /// See [`NoteReceiver::link_up`].
    pub fn link_up(&mut self, profile: LinkProfile, now: u64) -> Result<Vec<Frame>, CoreError> {
        let effects = self.machine.link_up(profile);
        self.apply(effects, now)
    }

    /// See [`NoteReceiver::link_down`].
    pub fn link_down(&mut self, now: u64) -> Result<(), CoreError> {
        let effects = self.machine.link_down();
        self.apply(effects, now).map(drop)
    }

    /// See [`NoteReceiver::on_frame`].
    pub fn on_frame(&mut self, frame: Frame, now: u64) -> Result<Vec<Frame>, CoreError> {
        let effects = self.machine.on_frame(frame, now);
        self.apply(effects, now)
    }

    /// See [`NoteReceiver::tick`].
    pub fn tick(&mut self, now: u64) -> Result<Vec<Frame>, CoreError> {
        let effects = self.machine.tick(now);
        self.apply(effects, now)
    }

    /// See [`NoteReceiver::set_free_bytes`].
    pub fn set_free_bytes(&mut self, free_bytes: Option<u64>) {
        self.free_bytes = free_bytes;
        self.machine.set_free_bytes(free_bytes);
    }

    /// Speech jobs to run since the last call. Run them away from the link and pass each
    /// result to [`complete_speech`](CompanionNotes::complete_speech).
    pub fn take_speech_jobs(&mut self) -> Vec<SpeechJob> {
        std::mem::take(&mut self.jobs)
    }

    /// A speech job finished.
    pub fn complete_speech(
        &mut self,
        outcome: SpeechOutcome,
        now: u64,
    ) -> Result<Vec<Frame>, CoreError> {
        let effects = match outcome {
            SpeechOutcome::Transcribed {
                note_id,
                attachment_id,
                result,
            } => self.machine.on_transcribed(note_id, attachment_id, result),
            SpeechOutcome::Synthesized {
                request_id,
                note_id,
                result,
            } => self.machine.on_synthesized(request_id, note_id, result),
        };
        self.apply(effects, now)
    }

    /// The state machine, to read notes from.
    pub fn machine(&self) -> &NoteReceiver {
        &self.machine
    }

    /// Where a verified attachment is.
    pub fn attachment_path(&self, id: &NoteId, attachment: AttachmentId) -> PathBuf {
        self.store.attachment_path(id, attachment)
    }

    /// Events since the last call.
    pub fn take_events(&mut self) -> Vec<ReceiverEvent> {
        std::mem::take(&mut self.events)
    }

    /// Notes whose records could not be read at open, kept aside on disk.
    pub fn set_aside(&self) -> &[String] {
        &self.set_aside
    }

    fn apply(&mut self, effects: Vec<ReceiverEffect>, now: u64) -> Result<Vec<Frame>, CoreError> {
        let result = self.run_effects(effects, now);
        if result.is_err() {
            let (records, _) = load_reconciled(&self.store)?;
            self.machine = NoteReceiver::new(self.retry, records);
            self.machine.set_free_bytes(self.free_bytes);
        }
        result
    }

    fn run_effects(
        &mut self,
        effects: Vec<ReceiverEffect>,
        now: u64,
    ) -> Result<Vec<Frame>, CoreError> {
        let mut queue: VecDeque<_> = effects.into();
        let mut frames = Vec::new();
        while let Some(effect) = queue.pop_front() {
            let more = match effect {
                ReceiverEffect::Send(frame) => {
                    frames.push(frame);
                    continue;
                }
                ReceiverEffect::Event(event) => {
                    self.events.push(event);
                    continue;
                }
                ReceiverEffect::Persist(record) => {
                    self.store.save(record.as_ref())?;
                    continue;
                }
                ReceiverEffect::TruncatePartial {
                    note_id,
                    attachment_id,
                    len,
                } => match self.store.truncate_partial(&note_id, attachment_id, len) {
                    Ok(()) => continue,
                    Err(e) => self.storage_failed(note_id, &e, &mut queue),
                },
                ReceiverEffect::WriteChunk {
                    note_id,
                    attachment_id,
                    offset,
                    data,
                } => match self
                    .store
                    .write_partial(&note_id, attachment_id, offset, &data)
                {
                    Ok(()) => continue,
                    Err(e) => self.storage_failed(note_id, &e, &mut queue),
                },
                ReceiverEffect::SyncPartial {
                    note_id,
                    attachment_id,
                } => match self.store.sync_partial(&note_id, attachment_id) {
                    Ok(()) => continue,
                    Err(e) => self.storage_failed(note_id, &e, &mut queue),
                },
                ReceiverEffect::VerifyAttachment {
                    note_id,
                    attachment_id,
                } => match self.store.hash_partial(&note_id, attachment_id) {
                    Ok(actual) => {
                        self.machine
                            .on_attachment_hashed(note_id, attachment_id, actual, now)
                    }
                    Err(e) => self.storage_failed(note_id, &e, &mut queue),
                },
                ReceiverEffect::CommitAttachment {
                    note_id,
                    attachment_id,
                } => {
                    // Failing here must stop the Persist and the ack queued behind it.
                    self.store.commit_partial(&note_id, attachment_id)?;
                    continue;
                }
                ReceiverEffect::Transcribe {
                    note_id,
                    attachment_id,
                    locale,
                    mime,
                } => {
                    self.jobs.push(SpeechJob::Transcribe(TranscribeRequest {
                        note_id,
                        attachment_id,
                        audio_path: self.store.attachment_path(&note_id, attachment_id),
                        mime,
                        locale,
                    }));
                    continue;
                }
                ReceiverEffect::Synthesize {
                    request_id,
                    note_id,
                    text,
                    locale,
                } => {
                    self.jobs.push(SpeechJob::Synthesize {
                        request_id,
                        request: SynthesizeRequest {
                            note_id,
                            text,
                            locale,
                        },
                    });
                    continue;
                }
            };
            for effect in more.into_iter().rev() {
                queue.push_front(effect);
            }
        }
        Ok(frames)
    }

    fn storage_failed(
        &mut self,
        note_id: NoteId,
        error: &std::io::Error,
        queue: &mut VecDeque<ReceiverEffect>,
    ) -> Vec<ReceiverEffect> {
        warn!(error = %error, "note storage failed; pausing the transfer");
        // What was queued behind this write assumed it worked.
        queue.clear();
        self.machine.on_storage_error(note_id)
    }
}

fn load_reconciled(store: &NoteStore) -> Result<(Vec<IncomingNote>, Vec<String>), CoreError> {
    let loaded = store.load::<IncomingNote>()?;
    let mut records = loaded.records;
    for record in &mut records {
        reconcile(store, record)?;
    }
    Ok((records, loaded.set_aside))
}

/// Bring a record in line with its files after a crash. The two can disagree in one
/// direction only, because data is always made durable before the record that claims it:
/// the commit rename may have happened with the crash before the record said so.
fn reconcile(store: &NoteStore, record: &mut IncomingNote) -> Result<(), CoreError> {
    let id = record.manifest.id;
    let mut changed = false;
    for meta in &record.manifest.attachments {
        let progress = record.attachments.entry(meta.id).or_default();
        if progress.verified {
            continue;
        }
        if store.attachment_path(&id, meta.id).exists() {
            if store.hash_attachment(&id, meta.id).ok() == Some(meta.sha256) {
                progress.verified = true;
                progress.committed_offset = meta.size;
                changed = true;
                continue;
            }
            store.remove_attachment(&id, meta.id)?;
        }
        // The file can lag the record only if storage lied about a sync. Trust the file.
        let on_disk = std::fs::metadata(store.partial_path(&id, meta.id))
            .map(|m| m.len())
            .unwrap_or(0);
        if progress.committed_offset > on_disk {
            progress.committed_offset = on_disk;
            changed = true;
        }
    }
    if changed {
        store.save(&*record)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::{manifest_digest, record::RECORD_FORMAT, sha256};
    use omg_protocol::{AttachmentMeta, NoteManifest, NoteOffer};
    use std::collections::BTreeMap;

    const ID: NoteId = NoteId([3; 16]);

    fn incoming(data: &[u8], committed: u64) -> IncomingNote {
        let manifest = NoteManifest {
            id: ID,
            created_at_ms: 0,
            locale: Locale::It,
            text: None,
            attachments: vec![AttachmentMeta {
                id: 0,
                kind: AttachmentKind::Photo,
                mime: "x/y".into(),
                size: data.len() as u64,
                sha256: sha256(data),
                duration_ms: None,
            }],
        };
        IncomingNote {
            format: RECORD_FORMAT,
            manifest_sha256: manifest_digest(&manifest),
            manifest,
            attachments: BTreeMap::from([(
                0,
                crate::notes::record::IncomingAttachment {
                    committed_offset: committed,
                    verified: false,
                    hash_failures: 0,
                },
            )]),
            complete: false,
            transcripts: BTreeMap::new(),
            received_at_ms: 0,
        }
    }

    #[test]
    fn crash_between_commit_rename_and_record_is_recognized() {
        let tmp = tempfile::tempdir().unwrap();
        let store = NoteStore::open(tmp.path()).unwrap();
        store.save(&incoming(b"data", 2)).unwrap();
        std::fs::write(store.attachment_path(&ID, 0), b"data").unwrap();

        let notes = CompanionNotes::open(tmp.path(), RetryPolicy::default()).unwrap();
        let held = notes.machine().note(&ID).unwrap();
        assert!(held.attachments[&0].verified);
        assert_eq!(held.attachments[&0].committed_offset, 4);
    }

    #[test]
    fn a_final_file_that_fails_its_digest_is_dropped_at_open() {
        let tmp = tempfile::tempdir().unwrap();
        let store = NoteStore::open(tmp.path()).unwrap();
        store.save(&incoming(b"data", 0)).unwrap();
        std::fs::write(store.attachment_path(&ID, 0), b"oops").unwrap();

        let notes = CompanionNotes::open(tmp.path(), RetryPolicy::default()).unwrap();
        assert!(!notes.machine().note(&ID).unwrap().attachments[&0].verified);
        assert!(!store.attachment_path(&ID, 0).exists());
    }

    #[test]
    fn a_partial_shorter_than_its_checkpoint_rewinds_and_a_longer_one_is_cut() {
        let tmp = tempfile::tempdir().unwrap();
        let store = NoteStore::open(tmp.path()).unwrap();
        store.save(&incoming(b"0123456789", 8)).unwrap();
        store.truncate_partial(&ID, 0, 5).unwrap();

        let mut notes = CompanionNotes::open(tmp.path(), RetryPolicy::default()).unwrap();
        assert_eq!(
            notes.machine().note(&ID).unwrap().attachments[&0].committed_offset,
            5
        );

        // A tail past the checkpoint goes on resume.
        store.truncate_partial(&ID, 0, 9).unwrap();
        let manifest = notes.machine().note(&ID).unwrap().manifest.clone();
        notes.link_up(LinkProfile::fast(), 0).unwrap();
        notes
            .on_frame(Frame::NoteOffer(NoteOffer { manifest }), 0)
            .unwrap();
        assert_eq!(
            std::fs::metadata(store.partial_path(&ID, 0)).unwrap().len(),
            5
        );
    }

    #[test]
    fn a_failing_write_pauses_instead_of_acknowledging() {
        let tmp = tempfile::tempdir().unwrap();
        let mut notes = CompanionNotes::open(tmp.path(), RetryPolicy::default()).unwrap();
        notes.link_up(LinkProfile::fast(), 0).unwrap();
        let manifest = incoming(b"data", 0).manifest;
        notes
            .on_frame(Frame::NoteOffer(NoteOffer { manifest }), 0)
            .unwrap();
        // The partial file vanishing makes the next write fail, as a full disk would.
        let store = NoteStore::open(tmp.path()).unwrap();
        std::fs::remove_file(store.partial_path(&ID, 0)).unwrap();

        let frames = notes
            .on_frame(
                Frame::NoteChunk(omg_protocol::NoteChunk {
                    note_id: ID,
                    attachment_id: 0,
                    offset: 0,
                    data: b"data".to_vec(),
                    sha256: sha256(b"data"),
                }),
                1,
            )
            .unwrap();
        assert!(frames.iter().all(|f| !matches!(
            f,
            Frame::NoteAttachmentAck(_) | Frame::NoteAck(_) | Frame::NoteChunkRequest(_)
        )));
        assert!(frames.iter().any(|f| matches!(
            f,
            Frame::NoteOfferReply(r) if matches!(
                r.status,
                omg_protocol::NoteOfferStatus::Deferred {
                    reason: omg_protocol::NoteDeferReason::InsufficientStorage
                }
            )
        )));
    }

    /// A store that fails while finishing a file must not leave a "verified" in memory that
    /// a later repeat offer would turn into an acknowledgement.
    #[cfg(unix)]
    #[test]
    fn a_store_failure_acknowledges_nothing_and_memory_falls_back_to_the_disk() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let mut notes = CompanionNotes::open(tmp.path(), RetryPolicy::default()).unwrap();
        notes.link_up(LinkProfile::fast(), 0).unwrap();
        let manifest = incoming(b"data", 0).manifest;
        let offer = Frame::NoteOffer(NoteOffer {
            manifest: manifest.clone(),
        });
        notes.on_frame(offer.clone(), 0).unwrap();

        let dir = tmp.path().join(ID.to_hex());
        let set_mode = |mode| {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        set_mode(0o555);
        if std::fs::File::create(dir.join("probe")).is_ok() {
            return; // Running as root: permissions do not bind, so nothing to test.
        }
        let chunk = Frame::NoteChunk(omg_protocol::NoteChunk {
            note_id: ID,
            attachment_id: 0,
            offset: 0,
            data: b"data".to_vec(),
            sha256: sha256(b"data"),
        });
        let failed = notes.on_frame(chunk.clone(), 1);
        set_mode(0o755);

        assert!(failed.is_err(), "and so no frames to send");
        let held = notes.machine().note(&ID).unwrap();
        assert!(!held.attachments[&0].verified);
        assert!(!held.complete);

        // The next link carries on and gets there honestly.
        notes.link_up(LinkProfile::fast(), 2).unwrap();
        let frames = notes.on_frame(offer, 2).unwrap();
        assert!(!frames
            .iter()
            .any(|f| matches!(f, Frame::NoteAttachmentAck(_))));
        let frames = notes.on_frame(chunk, 3).unwrap();
        assert!(frames.iter().any(|f| matches!(
            f,
            Frame::NoteAttachmentAck(a) if a.verified
        )));
        assert!(frames.iter().any(|f| matches!(f, Frame::NoteAck(_))));
    }

    #[test]
    fn glass_queue_survives_reopen_and_a_verified_note_is_not_reoffered() {
        let tmp = tempfile::tempdir().unwrap();
        let draft = NoteDraft {
            created_at_ms: 0,
            locale: Locale::It,
            text: Some("hello".into()),
            attachments: vec![],
        };
        let (a, b);
        {
            let mut g = GlassNotes::open(tmp.path(), SenderConfig::default()).unwrap();
            (a, _) = g.add_note(&draft, 0).unwrap();
            (b, _) = g.add_note(&draft, 0).unwrap();
        }
        let mut g = GlassNotes::open(tmp.path(), SenderConfig::default()).unwrap();
        let order: Vec<NoteId> = g.machine().notes().iter().map(|r| r.manifest.id).collect();
        assert_eq!(order, vec![a, b], "the order survives too");
        let frames = g.link_up(LinkProfile::fast(), 1).unwrap();
        let digest = g.machine().note(&a).unwrap().manifest_sha256;
        assert!(frames
            .iter()
            .any(|f| matches!(f, Frame::NoteOffer(o) if o.manifest.id == a)));

        g.on_frame(
            Frame::NoteAck(omg_protocol::NoteAck {
                note_id: a,
                manifest_sha256: digest,
            }),
            2,
        )
        .unwrap();
        drop(g);

        let mut g = GlassNotes::open(tmp.path(), SenderConfig::default()).unwrap();
        let frames = g.link_up(LinkProfile::fast(), 3).unwrap();
        assert!(frames
            .iter()
            .any(|f| matches!(f, Frame::NoteOffer(o) if o.manifest.id == b)));
        assert!(!frames
            .iter()
            .any(|f| matches!(f, Frame::NoteOffer(o) if o.manifest.id == a)));
    }
}
