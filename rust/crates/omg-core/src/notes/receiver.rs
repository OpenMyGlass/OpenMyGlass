//! The companion side of note sync: a pure state machine.
//!
//! It pulls one attachment at a time, in order within the file: the recording first so a
//! transcript comes early, the smaller files before the bigger. Every chunk is checked
//! before it is written and every file is checked whole before it is acknowledged. It
//! touches nothing itself: every call returns [`ReceiverEffect`]s to carry out in order,
//! and the order is the crash-safety argument: `verified` reaches the Glass only after the
//! file has been committed and the record persisted.

use std::collections::{btree_map::Entry, BTreeMap, BTreeSet};

use omg_protocol::{
    AttachmentId, AttachmentKind, AttachmentMeta, AttachmentProgress, Frame, Locale, NoteAck,
    NoteAttachmentAck, NoteChunk, NoteChunkReject, NoteChunkRejectReason, NoteChunkRequest,
    NoteChunkUnavailable, NoteDeferReason, NoteId, NoteOffer, NoteOfferReply, NoteOfferStatus,
    NoteRejectReason, NoteSyncStatus, NoteTranscript, NoteTranscriptAck, NoteTranscriptError,
    NoteTranscriptRequest, NoteTtsChunk, NoteTtsChunkRequest, NoteTtsFailed, NoteTtsReady,
    NoteTtsRelease, NoteTtsRequest, Sha256Digest, MAX_NOTE_CHUNK_LEN, MAX_TRANSCRIPT_BYTES,
    MAX_TTS_AUDIO_BYTES, MAX_TTS_TEXT_BYTES,
};

use super::{
    manifest_digest,
    record::{IncomingAttachment, IncomingNote, StoredTranscript, TranscriptRecord, RECORD_FORMAT},
    sha256,
    speech::{SpeechError, SynthesizedAudio},
    LinkProfile, RetryPolicy,
};

/// Progress is made durable at least this often. A crash re-fetches at most this much.
const CHECKPOINT_BYTES: u64 = 1024 * 1024;
/// Rejected chunks in a row before the transfer pauses.
const MAX_CONSECUTIVE_REJECTS: u32 = 8;
/// Whole-file failures before the transfer pauses and the Glass is told to look.
const MAX_HASH_FAILURES: u32 = 3;
/// Silent timeouts in a row before the transfer stops waiting. The next offer resumes it.
const MAX_STALLS: u32 = 3;

/// Something that happened, for the companion UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiverEvent {
    /// A new note was accepted.
    NoteStarted {
        /// The note.
        note_id: NoteId,
    },
    /// One attachment is durable and verified.
    AttachmentVerified {
        /// The note.
        note_id: NoteId,
        /// The attachment.
        attachment_id: AttachmentId,
    },
    /// The whole note is durable and verified.
    NoteComplete {
        /// The note.
        note_id: NoteId,
    },
    /// A chunk was thrown away; it will be requested again.
    ChunkRejected {
        /// The note.
        note_id: NoteId,
        /// The attachment.
        attachment_id: AttachmentId,
        /// The chunk's offset.
        offset: u64,
        /// Why.
        reason: NoteChunkRejectReason,
    },
    /// A whole file failed its checksum and is being fetched again.
    AttachmentHashFailed {
        /// The note.
        note_id: NoteId,
        /// The attachment.
        attachment_id: AttachmentId,
    },
    /// The transfer paused; the note stays on the Glass.
    NoteDeferred {
        /// The note.
        note_id: NoteId,
        /// Why.
        reason: NoteDeferReason,
    },
    /// A note was refused.
    NoteRejected {
        /// The note.
        note_id: NoteId,
        /// Why.
        reason: NoteRejectReason,
    },
    /// A transcript was stored, or the engine failed.
    TranscriptReady {
        /// The note.
        note_id: NoteId,
        /// The voice attachment.
        attachment_id: AttachmentId,
        /// The result.
        result: StoredTranscript,
    },
}

/// Something the caller must do, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum ReceiverEffect {
    /// Send this frame to the Glass.
    Send(Frame),
    /// Store this record durably.
    Persist(Box<IncomingNote>),
    /// Set the partial file to exactly this length, creating it if needed. Clears any tail
    /// a crash left past the last checkpoint.
    TruncatePartial {
        /// The note.
        note_id: NoteId,
        /// The attachment.
        attachment_id: AttachmentId,
        /// New length.
        len: u64,
    },
    /// Write checked bytes into the partial file. On failure call
    /// [`NoteReceiver::on_storage_error`] and drop the effects after this one.
    WriteChunk {
        /// The note.
        note_id: NoteId,
        /// The attachment.
        attachment_id: AttachmentId,
        /// Where.
        offset: u64,
        /// What.
        data: Vec<u8>,
    },
    /// Make the partial file's contents durable.
    SyncPartial {
        /// The note.
        note_id: NoteId,
        /// The attachment.
        attachment_id: AttachmentId,
    },
    /// Hash the partial file and call [`NoteReceiver::on_attachment_hashed`].
    VerifyAttachment {
        /// The note.
        note_id: NoteId,
        /// The attachment.
        attachment_id: AttachmentId,
    },
    /// Durably move the partial file to its final place.
    CommitAttachment {
        /// The note.
        note_id: NoteId,
        /// The attachment.
        attachment_id: AttachmentId,
    },
    /// Run speech-to-text off the link thread, then call [`NoteReceiver::on_transcribed`].
    Transcribe {
        /// The note.
        note_id: NoteId,
        /// The voice attachment.
        attachment_id: AttachmentId,
        /// Language.
        locale: Locale,
        /// MIME type.
        mime: String,
    },
    /// Run TTS off the link thread, then call [`NoteReceiver::on_synthesized`].
    Synthesize {
        /// The request.
        request_id: u32,
        /// The note.
        note_id: NoteId,
        /// What to say.
        text: String,
        /// Language.
        locale: Locale,
    },
    /// Tell the UI.
    Event(ReceiverEvent),
}

struct Active {
    note_id: NoteId,
    attachment_id: AttachmentId,
    size: u64,
    /// The next byte to accept.
    next_offset: u64,
    /// The next byte to ask for.
    requested_to: u64,
    /// Bytes accepted since the last checkpoint.
    since_checkpoint: u64,
    /// After a rejection, chunks from the old window are still on their way: ignore them
    /// until the one at `next_offset` arrives.
    rewinding: bool,
    rejects: u32,
    stalls: u32,
    deadline: u64,
}

struct TtsAudio {
    request_id: u32,
    ready: NoteTtsReady,
    pcm: Vec<u8>,
}

/// See the module docs.
pub struct NoteReceiver {
    retry: RetryPolicy,
    notes: BTreeMap<NoteId, IncomingNote>,
    profile: Option<LinkProfile>,
    active: Option<Active>,
    free_bytes: Option<u64>,
    running_stt: BTreeSet<(NoteId, AttachmentId)>,
    running_tts: Option<u32>,
    tts: Option<TtsAudio>,
    peer_status: Option<NoteSyncStatus>,
}

impl NoteReceiver {
    /// Rebuild from what the store holds.
    pub fn new(retry: RetryPolicy, records: Vec<IncomingNote>) -> Self {
        NoteReceiver {
            retry,
            notes: records.into_iter().map(|r| (r.manifest.id, r)).collect(),
            profile: None,
            active: None,
            free_bytes: None,
            running_stt: BTreeSet::new(),
            running_tts: None,
            tts: None,
            peer_status: None,
        }
    }

    /// Space left for notes, as the platform reports it. `None` means unknown: offers are
    /// then only limited by real write failures.
    pub fn set_free_bytes(&mut self, free_bytes: Option<u64>) {
        self.free_bytes = free_bytes;
    }

    /// A record held here.
    pub fn note(&self, id: &NoteId) -> Option<&IncomingNote> {
        self.notes.get(id)
    }

    /// All records held here.
    pub fn notes(&self) -> impl Iterator<Item = &IncomingNote> {
        self.notes.values()
    }

    /// What the Glass last said is waiting there.
    pub fn peer_status(&self) -> Option<NoteSyncStatus> {
        self.peer_status
    }

    /// Nothing left to do on this link: the Glass has nothing on offer, no speech job is
    /// running and every transcript has been delivered.
    pub fn is_idle(&self) -> bool {
        self.peer_status.is_some_and(|s| !s.sending)
            && self.active.is_none()
            && self.running_stt.is_empty()
            && self.running_tts.is_none()
            && self.tts.is_none()
            && self
                .notes
                .values()
                .flat_map(|n| n.transcripts.values())
                .all(|t| t.acked || t.result.is_none())
    }

    /// The earliest time [`tick`](NoteReceiver::tick) has something to do.
    pub fn next_deadline(&self) -> Option<u64> {
        self.active.as_ref().map(|a| a.deadline)
    }

    /// A link is up and authenticated, and the peer advertised the notes capability.
    /// Undelivered transcripts go out, and interrupted transcriptions start again.
    pub fn link_up(&mut self, profile: LinkProfile) -> Vec<ReceiverEffect> {
        self.profile = Some(profile.normalized());
        self.active = None;
        self.peer_status = None;
        let mut out = Vec::new();
        for note in self.notes.values() {
            for (attachment_id, record) in &note.transcripts {
                match &record.result {
                    Some(result) if !record.acked => {
                        out.push(ReceiverEffect::Send(transcript_frame(
                            note.manifest.id,
                            *attachment_id,
                            result,
                        )));
                    }
                    None if self.running_stt.insert((note.manifest.id, *attachment_id)) => {
                        out.extend(transcribe_effect(note, *attachment_id));
                    }
                    _ => {}
                }
            }
        }
        out
    }

    /// The link went away. What was received so far becomes durable, so the transfer
    /// resumes exactly here. Results of speech jobs still running are lost with the link;
    /// the job runs again on the next one.
    pub fn link_down(&mut self) -> Vec<ReceiverEffect> {
        let mut out = Vec::new();
        self.checkpoint(&mut out);
        self.active = None;
        self.profile = None;
        self.running_stt.clear();
        self.running_tts = None;
        self.tts = None;
        out
    }

    /// Time passed.
    pub fn tick(&mut self, now: u64) -> Vec<ReceiverEffect> {
        let mut out = Vec::new();
        let timeout = self.retry.response_timeout_ms;
        let Some(active) = &mut self.active else {
            return out;
        };
        if now < active.deadline {
            return out;
        }
        active.stalls += 1;
        if active.stalls > MAX_STALLS {
            // Stop waiting. The Glass repeats its offer on its own timer, and that resumes.
            self.checkpoint(&mut out);
            self.active = None;
            return out;
        }
        active.deadline = now + timeout;
        active.requested_to = active.next_offset;
        active.rewinding = true;
        self.request_window(&mut out);
        out
    }

    /// A frame arrived. Frames that are not this side's are ignored.
    pub fn on_frame(&mut self, frame: Frame, now: u64) -> Vec<ReceiverEffect> {
        let mut out = Vec::new();
        match frame {
            Frame::NoteOffer(offer) => self.on_offer(offer, now, &mut out),
            Frame::NoteChunk(chunk) => self.on_chunk(chunk, now, &mut out),
            Frame::NoteChunkUnavailable(u) => self.on_unavailable(u, &mut out),
            Frame::NoteSyncStatus(status) => self.peer_status = Some(status),
            Frame::NoteTranscriptRequest(r) => self.on_transcript_request(r, &mut out),
            Frame::NoteTranscriptAck(ack) => self.on_transcript_ack(ack, &mut out),
            Frame::NoteTtsRequest(r) => self.on_tts_request(r, &mut out),
            Frame::NoteTtsChunkRequest(r) => self.on_tts_chunk_request(r, &mut out),
            Frame::NoteTtsRelease(r) => self.on_tts_release(r),
            _ => {}
        }
        out
    }

    /// Answer to [`ReceiverEffect::VerifyAttachment`]: the digest of what is on disk.
    pub fn on_attachment_hashed(
        &mut self,
        note_id: NoteId,
        attachment_id: AttachmentId,
        actual: Sha256Digest,
        now: u64,
    ) -> Vec<ReceiverEffect> {
        let mut out = Vec::new();
        let active_here = self
            .active
            .as_ref()
            .is_some_and(|a| a.note_id == note_id && a.attachment_id == attachment_id);
        if !active_here {
            return out;
        }
        let Some(note) = self.notes.get_mut(&note_id) else {
            return out;
        };
        let Some(meta) = note.manifest.attachment(attachment_id).cloned() else {
            return out;
        };
        let progress = note.attachments.entry(attachment_id).or_default();
        if actual == meta.sha256 {
            progress.verified = true;
            progress.committed_offset = meta.size;
            progress.hash_failures = 0;
            out.push(ReceiverEffect::CommitAttachment {
                note_id,
                attachment_id,
            });
            out.push(ReceiverEffect::Persist(Box::new(note.clone())));
            out.push(ReceiverEffect::Send(Frame::NoteAttachmentAck(
                NoteAttachmentAck {
                    note_id,
                    attachment_id,
                    sha256: actual,
                    verified: true,
                },
            )));
            out.push(ReceiverEffect::Event(ReceiverEvent::AttachmentVerified {
                note_id,
                attachment_id,
            }));
            self.active = None;
            self.start_next(note_id, now, &mut out);
            return out;
        }
        // The bytes all passed their chunk checks, yet the whole does not match. Start over.
        progress.committed_offset = 0;
        progress.hash_failures += 1;
        let give_up = progress.hash_failures >= MAX_HASH_FAILURES;
        if give_up {
            progress.hash_failures = 0;
        }
        out.push(ReceiverEffect::TruncatePartial {
            note_id,
            attachment_id,
            len: 0,
        });
        out.push(ReceiverEffect::Persist(Box::new(note.clone())));
        out.push(ReceiverEffect::Send(Frame::NoteAttachmentAck(
            NoteAttachmentAck {
                note_id,
                attachment_id,
                sha256: actual,
                verified: false,
            },
        )));
        out.push(ReceiverEffect::Event(ReceiverEvent::AttachmentHashFailed {
            note_id,
            attachment_id,
        }));
        self.active = None;
        if give_up {
            self.defer(note_id, NoteDeferReason::IntegrityFailure, &mut out);
        } else {
            self.start_next(note_id, now, &mut out);
        }
        out
    }

    /// A write failed, most likely for lack of space. The transfer pauses; what was
    /// checkpointed stays. The Glass keeps the note and offers it again later.
    pub fn on_storage_error(&mut self, note_id: NoteId) -> Vec<ReceiverEffect> {
        let mut out = Vec::new();
        if self.active.as_ref().map(|a| a.note_id) == Some(note_id) {
            self.active = None;
            self.defer(note_id, NoteDeferReason::InsufficientStorage, &mut out);
        }
        out
    }

    /// Answer to [`ReceiverEffect::Transcribe`].
    pub fn on_transcribed(
        &mut self,
        note_id: NoteId,
        attachment_id: AttachmentId,
        result: Result<String, SpeechError>,
    ) -> Vec<ReceiverEffect> {
        let mut out = Vec::new();
        self.running_stt.remove(&(note_id, attachment_id));
        let Some(note) = self.notes.get_mut(&note_id) else {
            return out;
        };
        let stored = match result {
            Ok(text) => StoredTranscript { text, error: None },
            Err(SpeechError::Unavailable) => StoredTranscript {
                text: String::new(),
                error: Some(NoteTranscriptError::EngineUnavailable),
            },
            Err(SpeechError::Failed(why)) => StoredTranscript {
                text: String::new(),
                error: Some(NoteTranscriptError::EngineFailed(why)),
            },
        };
        note.transcripts.insert(
            attachment_id,
            TranscriptRecord {
                result: Some(stored.clone()),
                acked: false,
            },
        );
        out.push(ReceiverEffect::Persist(Box::new(note.clone())));
        out.push(ReceiverEffect::Event(ReceiverEvent::TranscriptReady {
            note_id,
            attachment_id,
            result: stored.clone(),
        }));
        if self.profile.is_some() {
            out.push(ReceiverEffect::Send(transcript_frame(
                note_id,
                attachment_id,
                &stored,
            )));
        }
        out
    }

    /// Answer to [`ReceiverEffect::Synthesize`].
    pub fn on_synthesized(
        &mut self,
        request_id: u32,
        note_id: NoteId,
        result: Result<SynthesizedAudio, SpeechError>,
    ) -> Vec<ReceiverEffect> {
        let mut out = Vec::new();
        if self.running_tts != Some(request_id) {
            return out; // Replaced, or the link went away.
        }
        self.running_tts = None;
        let failure = match result {
            Ok(audio) if audio.pcm.is_empty() => Some("the engine returned no audio".to_string()),
            Ok(audio) if audio.pcm.len() as u64 > MAX_TTS_AUDIO_BYTES => {
                Some("the audio is too long".to_string())
            }
            Ok(audio) => {
                let ready = NoteTtsReady {
                    request_id,
                    note_id,
                    sample_rate: audio.sample_rate,
                    total_len: audio.pcm.len() as u64,
                    sha256: sha256(&audio.pcm),
                };
                out.push(ReceiverEffect::Send(Frame::NoteTtsReady(ready)));
                self.tts = Some(TtsAudio {
                    request_id,
                    ready,
                    pcm: audio.pcm,
                });
                None
            }
            Err(e) => Some(e.to_string()),
        };
        if let Some(reason) = failure {
            out.push(tts_failed(request_id, note_id, reason));
        }
        out
    }

    fn on_offer(&mut self, offer: NoteOffer, now: u64, out: &mut Vec<ReceiverEffect>) {
        let manifest = offer.manifest;
        let note_id = manifest.id;
        let reject = |reason, out: &mut Vec<ReceiverEffect>| {
            out.push(offer_reply(note_id, NoteOfferStatus::Rejected { reason }));
            out.push(ReceiverEffect::Event(ReceiverEvent::NoteRejected {
                note_id,
                reason,
            }));
        };
        if manifest.validate().is_err() {
            reject(NoteRejectReason::InvalidManifest, out);
            return;
        }
        let digest = manifest_digest(&manifest);
        if let Some(existing) = self.notes.get(&note_id) {
            if existing.manifest_sha256 != digest {
                reject(NoteRejectReason::ManifestConflict, out);
                return;
            }
            // A repeat. Lost acknowledgements are sent again; nothing is reset.
            self.resend_attachment_acks(note_id, out);
            if self.notes[&note_id].complete {
                out.push(offer_reply(
                    note_id,
                    NoteOfferStatus::AlreadyComplete {
                        manifest_sha256: digest,
                    },
                ));
                return;
            }
            if self.active.as_ref().map(|a| a.note_id) == Some(note_id) {
                let progress = self.progress(&note_id);
                out.push(offer_reply(note_id, NoteOfferStatus::Accepted { progress }));
                return;
            }
        }
        // The Glass offers one note at a time, so an offer of another note means it has given
        // up on the one in progress. Late chunks for that one will simply be ignored.
        self.checkpoint(out);
        self.active = None;
        let already_held: u64 = self.notes.get(&note_id).map_or(0, |note| {
            note.attachments.values().map(|a| a.committed_offset).sum()
        });
        let total: u64 = manifest.attachments.iter().map(|a| a.size).sum();
        if self
            .free_bytes
            .is_some_and(|free| total.saturating_sub(already_held) > free)
        {
            self.defer(note_id, NoteDeferReason::InsufficientStorage, out);
            return;
        }
        if let Entry::Vacant(slot) = self.notes.entry(note_id) {
            let attachments = manifest
                .attachments
                .iter()
                .map(|a| (a.id, IncomingAttachment::default()))
                .collect();
            let record = IncomingNote {
                format: RECORD_FORMAT,
                manifest,
                manifest_sha256: digest,
                attachments,
                complete: false,
                transcripts: BTreeMap::new(),
                received_at_ms: now,
            };
            // Durable before the Glass hears it was accepted.
            out.push(ReceiverEffect::Persist(Box::new(record.clone())));
            out.push(ReceiverEffect::Event(ReceiverEvent::NoteStarted {
                note_id,
            }));
            slot.insert(record);
        }
        let progress = self.progress(&note_id);
        out.push(offer_reply(note_id, NoteOfferStatus::Accepted { progress }));
        self.start_next(note_id, now, out);
    }

    fn progress(&self, note_id: &NoteId) -> Vec<AttachmentProgress> {
        let Some(note) = self.notes.get(note_id) else {
            return Vec::new();
        };
        note.manifest
            .attachments
            .iter()
            .map(|meta| {
                let held = note.attachments.get(&meta.id).copied().unwrap_or_default();
                let next_offset = match &self.active {
                    Some(a) if a.note_id == *note_id && a.attachment_id == meta.id => a.next_offset,
                    _ => held.committed_offset,
                };
                AttachmentProgress {
                    attachment_id: meta.id,
                    next_offset,
                    verified: held.verified,
                }
            })
            .collect()
    }

    fn resend_attachment_acks(&self, note_id: NoteId, out: &mut Vec<ReceiverEffect>) {
        let Some(note) = self.notes.get(&note_id) else {
            return;
        };
        for meta in &note.manifest.attachments {
            if note.attachments.get(&meta.id).is_some_and(|a| a.verified) {
                out.push(ReceiverEffect::Send(Frame::NoteAttachmentAck(
                    NoteAttachmentAck {
                        note_id,
                        attachment_id: meta.id,
                        sha256: meta.sha256,
                        verified: true,
                    },
                )));
            }
        }
    }

    /// Start the next attachment of this note, or finish the note.
    fn start_next(&mut self, note_id: NoteId, now: u64, out: &mut Vec<ReceiverEffect>) {
        let Some(profile) = self.profile else {
            return;
        };
        let Some(note) = self.notes.get_mut(&note_id) else {
            return;
        };
        let mut todo: Vec<&AttachmentMeta> = note
            .manifest
            .attachments
            .iter()
            .filter(|a| !note.attachments.get(&a.id).is_some_and(|p| p.verified))
            .collect();
        if todo.is_empty() {
            note.complete = true;
            let manifest_sha256 = note.manifest_sha256;
            out.push(ReceiverEffect::Persist(Box::new(note.clone())));
            out.push(ReceiverEffect::Send(Frame::NoteAck(NoteAck {
                note_id,
                manifest_sha256,
            })));
            out.push(ReceiverEffect::Event(ReceiverEvent::NoteComplete {
                note_id,
            }));
            self.active = None;
            return;
        }
        todo.sort_by_key(|a| (kind_rank(a.kind), a.size, a.id));
        let within_limit = |a: &&&AttachmentMeta| {
            profile
                .max_auto_attachment_bytes
                .is_none_or(|limit| a.size <= limit)
        };
        let Some(meta) = todo.iter().find(within_limit) else {
            let bytes = todo[0].size;
            self.defer(note_id, NoteDeferReason::NeedsFasterLink { bytes }, out);
            return;
        };
        let (attachment_id, size) = (meta.id, meta.size);
        let held = note
            .attachments
            .get(&attachment_id)
            .map_or(0, |p| p.committed_offset);
        out.push(ReceiverEffect::TruncatePartial {
            note_id,
            attachment_id,
            len: held,
        });
        self.active = Some(Active {
            note_id,
            attachment_id,
            size,
            next_offset: held,
            requested_to: held,
            since_checkpoint: 0,
            rewinding: false,
            rejects: 0,
            stalls: 0,
            deadline: now + self.retry.response_timeout_ms,
        });
        if held >= size {
            // All bytes arrived before the link dropped; only the check is left.
            out.push(ReceiverEffect::VerifyAttachment {
                note_id,
                attachment_id,
            });
        } else {
            self.request_window(out);
        }
    }

    fn request_window(&mut self, out: &mut Vec<ReceiverEffect>) {
        let Some(profile) = self.profile else {
            return;
        };
        let Some(active) = &mut self.active else {
            return;
        };
        let chunk = u64::from(profile.chunk_len);
        let window = chunk * u64::from(profile.window);
        while active.requested_to < active.size && active.requested_to - active.next_offset < window
        {
            out.push(ReceiverEffect::Send(Frame::NoteChunkRequest(
                NoteChunkRequest {
                    note_id: active.note_id,
                    attachment_id: active.attachment_id,
                    offset: active.requested_to,
                    max_len: profile.chunk_len,
                },
            )));
            active.requested_to += chunk.min(active.size - active.requested_to);
        }
    }

    fn on_chunk(&mut self, chunk: NoteChunk, now: u64, out: &mut Vec<ReceiverEffect>) {
        let timeout = self.retry.response_timeout_ms;
        let Some(profile) = self.profile else {
            return;
        };
        let Some(active) = self
            .active
            .as_mut()
            .filter(|a| a.note_id == chunk.note_id && a.attachment_id == chunk.attachment_id)
        else {
            return; // Nothing we are waiting for.
        };
        let (note_id, attachment_id) = (active.note_id, active.attachment_id);
        active.deadline = now + timeout;
        active.stalls = 0;
        if chunk.offset < active.next_offset {
            return; // A repeat of something already taken.
        }
        let reason = if chunk.offset > active.next_offset {
            if active.rewinding {
                return; // Old window, still draining.
            }
            Some(NoteChunkRejectReason::Gap)
        } else {
            let expected = u64::from(profile.chunk_len.min(MAX_NOTE_CHUNK_LEN))
                .min(active.size - active.next_offset);
            if chunk.data.len() as u64 != expected {
                Some(NoteChunkRejectReason::BadLength)
            } else if sha256(&chunk.data) != chunk.sha256 {
                Some(NoteChunkRejectReason::Corrupt)
            } else {
                None
            }
        };
        if let Some(reason) = reason {
            active.rejects += 1;
            active.rewinding = true;
            active.requested_to = active.next_offset;
            let give_up = active.rejects >= MAX_CONSECUTIVE_REJECTS;
            out.push(ReceiverEffect::Send(Frame::NoteChunkReject(
                NoteChunkReject {
                    note_id,
                    attachment_id,
                    offset: chunk.offset,
                    reason,
                },
            )));
            out.push(ReceiverEffect::Event(ReceiverEvent::ChunkRejected {
                note_id,
                attachment_id,
                offset: chunk.offset,
                reason,
            }));
            if give_up {
                self.checkpoint(out);
                self.active = None;
                self.defer(note_id, NoteDeferReason::IntegrityFailure, out);
            } else {
                self.request_window(out);
            }
            return;
        }
        let len = chunk.data.len() as u64;
        out.push(ReceiverEffect::WriteChunk {
            note_id,
            attachment_id,
            offset: chunk.offset,
            data: chunk.data,
        });
        active.next_offset += len;
        active.since_checkpoint += len;
        active.rewinding = false;
        active.rejects = 0;
        if active.next_offset >= active.size {
            self.checkpoint(out);
            out.push(ReceiverEffect::VerifyAttachment {
                note_id,
                attachment_id,
            });
        } else {
            if active.since_checkpoint >= CHECKPOINT_BYTES {
                self.checkpoint(out);
            }
            self.request_window(out);
        }
    }

    /// Data first, then the record that claims it.
    fn checkpoint(&mut self, out: &mut Vec<ReceiverEffect>) {
        let Some(active) = &mut self.active else {
            return;
        };
        let Some(note) = self.notes.get_mut(&active.note_id) else {
            return;
        };
        let progress = note.attachments.entry(active.attachment_id).or_default();
        if progress.committed_offset == active.next_offset {
            return;
        }
        out.push(ReceiverEffect::SyncPartial {
            note_id: active.note_id,
            attachment_id: active.attachment_id,
        });
        progress.committed_offset = active.next_offset;
        active.since_checkpoint = 0;
        out.push(ReceiverEffect::Persist(Box::new(note.clone())));
    }

    fn on_unavailable(&mut self, u: NoteChunkUnavailable, out: &mut Vec<ReceiverEffect>) {
        let here = self
            .active
            .as_ref()
            .is_some_and(|a| a.note_id == u.note_id && a.attachment_id == u.attachment_id);
        if here {
            // Stop pulling. If the Glass can serve this again it will offer it again.
            self.checkpoint(out);
            self.active = None;
        }
    }

    fn defer(&mut self, note_id: NoteId, reason: NoteDeferReason, out: &mut Vec<ReceiverEffect>) {
        out.push(offer_reply(note_id, NoteOfferStatus::Deferred { reason }));
        out.push(ReceiverEffect::Event(ReceiverEvent::NoteDeferred {
            note_id,
            reason,
        }));
    }

    fn on_transcript_request(
        &mut self,
        request: NoteTranscriptRequest,
        out: &mut Vec<ReceiverEffect>,
    ) {
        let NoteTranscriptRequest {
            note_id,
            attachment_id,
        } = request;
        let refuse = |error, out: &mut Vec<ReceiverEffect>| {
            out.push(ReceiverEffect::Send(Frame::NoteTranscript(
                NoteTranscript {
                    note_id,
                    attachment_id,
                    text: String::new(),
                    truncated: false,
                    error: Some(error),
                },
            )));
        };
        let Some(note) = self.notes.get_mut(&note_id) else {
            refuse(NoteTranscriptError::AudioNotAvailable, out);
            return;
        };
        let is_voice = note
            .manifest
            .attachment(attachment_id)
            .is_some_and(|a| a.kind == AttachmentKind::VoiceRecording);
        if !is_voice {
            refuse(NoteTranscriptError::NotAVoiceRecording, out);
            return;
        }
        if !note
            .attachments
            .get(&attachment_id)
            .is_some_and(|a| a.verified)
        {
            refuse(NoteTranscriptError::AudioNotAvailable, out);
            return;
        }
        if let Some(TranscriptRecord {
            result: Some(result),
            ..
        }) = note.transcripts.get(&attachment_id)
        {
            if result.error.is_none() {
                out.push(ReceiverEffect::Send(transcript_frame(
                    note_id,
                    attachment_id,
                    result,
                )));
                return;
            }
        }
        // First time, or the last attempt failed. Durable first, so a crash resumes it.
        note.transcripts
            .insert(attachment_id, TranscriptRecord::default());
        if self.running_stt.insert((note_id, attachment_id)) {
            out.push(ReceiverEffect::Persist(Box::new(note.clone())));
            out.extend(transcribe_effect(note, attachment_id));
        }
    }

    fn on_transcript_ack(&mut self, ack: NoteTranscriptAck, out: &mut Vec<ReceiverEffect>) {
        let Some(note) = self.notes.get_mut(&ack.note_id) else {
            return;
        };
        if let Some(record) = note.transcripts.get_mut(&ack.attachment_id) {
            if record.result.is_some() && !record.acked {
                record.acked = true;
                out.push(ReceiverEffect::Persist(Box::new(note.clone())));
            }
        }
    }

    fn on_tts_request(&mut self, request: NoteTtsRequest, out: &mut Vec<ReceiverEffect>) {
        let NoteTtsRequest {
            request_id,
            note_id,
            text,
            locale,
        } = request;
        if self.running_tts == Some(request_id) {
            return; // Already at it.
        }
        if let Some(tts) = self.tts.as_ref().filter(|t| t.request_id == request_id) {
            out.push(ReceiverEffect::Send(Frame::NoteTtsReady(tts.ready)));
            return; // A repeat: the audio is already here.
        }
        let text = match text {
            Some(text) => Some(text),
            None => self.notes.get(&note_id).and_then(spoken_text),
        };
        let Some(text) = text else {
            out.push(tts_failed(
                request_id,
                note_id,
                "the note has no text".to_string(),
            ));
            return;
        };
        if text.len() > MAX_TTS_TEXT_BYTES {
            out.push(tts_failed(
                request_id,
                note_id,
                "the text is too long".to_string(),
            ));
            return;
        }
        self.tts = None;
        self.running_tts = Some(request_id);
        out.push(ReceiverEffect::Synthesize {
            request_id,
            note_id,
            text,
            locale,
        });
    }

    fn on_tts_chunk_request(&mut self, r: NoteTtsChunkRequest, out: &mut Vec<ReceiverEffect>) {
        let Some(tts) = self.tts.as_ref().filter(|t| t.request_id == r.request_id) else {
            return;
        };
        let total = tts.pcm.len() as u64;
        if r.offset >= total || r.max_len == 0 {
            return;
        }
        let len = u64::from(r.max_len.min(MAX_NOTE_CHUNK_LEN)).min(total - r.offset) as usize;
        let start = r.offset as usize;
        let data = tts.pcm[start..start + len].to_vec();
        let sha256 = sha256(&data);
        out.push(ReceiverEffect::Send(Frame::NoteTtsChunk(NoteTtsChunk {
            request_id: r.request_id,
            offset: r.offset,
            data,
            sha256,
        })));
    }

    fn on_tts_release(&mut self, r: NoteTtsRelease) {
        if self.tts.as_ref().map(|t| t.request_id) == Some(r.request_id) {
            self.tts = None;
        }
        if self.running_tts == Some(r.request_id) {
            self.running_tts = None;
        }
    }
}

fn kind_rank(kind: AttachmentKind) -> u8 {
    match kind {
        AttachmentKind::VoiceRecording => 0,
        AttachmentKind::Photo => 1,
        AttachmentKind::Video => 2,
    }
}

fn offer_reply(note_id: NoteId, status: NoteOfferStatus) -> ReceiverEffect {
    ReceiverEffect::Send(Frame::NoteOfferReply(NoteOfferReply { note_id, status }))
}

fn tts_failed(request_id: u32, note_id: NoteId, reason: String) -> ReceiverEffect {
    ReceiverEffect::Send(Frame::NoteTtsFailed(NoteTtsFailed {
        request_id,
        note_id,
        reason,
    }))
}

fn transcribe_effect(note: &IncomingNote, attachment_id: AttachmentId) -> Option<ReceiverEffect> {
    let meta = note.manifest.attachment(attachment_id)?;
    Some(ReceiverEffect::Transcribe {
        note_id: note.manifest.id,
        attachment_id,
        locale: note.manifest.locale,
        mime: meta.mime.clone(),
    })
}

/// What "read this note" means: the typed text, else the transcript.
fn spoken_text(note: &IncomingNote) -> Option<String> {
    if let Some(text) = note.manifest.text.as_ref().filter(|t| !t.is_empty()) {
        return Some(text.clone());
    }
    note.transcripts
        .values()
        .filter_map(|t| t.result.as_ref())
        .find(|r| r.error.is_none() && !r.text.is_empty())
        .map(|r| r.text.clone())
}

/// The frame for a transcript: cut at the frame limit, on a character boundary. The
/// companion keeps the full text.
fn transcript_frame(
    note_id: NoteId,
    attachment_id: AttachmentId,
    stored: &StoredTranscript,
) -> Frame {
    let mut end = stored.text.len().min(MAX_TRANSCRIPT_BYTES);
    while !stored.text.is_char_boundary(end) {
        end -= 1;
    }
    Frame::NoteTranscript(NoteTranscript {
        note_id,
        attachment_id,
        text: stored.text[..end].to_string(),
        truncated: end < stored.text.len(),
        error: stored.error.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use omg_protocol::NoteManifest;

    const NOTE: NoteId = NoteId([1; 16]);

    fn profile(chunk_len: u32, window: u32, limit: Option<u64>) -> LinkProfile {
        LinkProfile {
            chunk_len,
            window,
            max_auto_attachment_bytes: limit,
        }
    }

    fn manifest(files: &[(AttachmentKind, &[u8])]) -> NoteManifest {
        NoteManifest {
            id: NOTE,
            created_at_ms: 0,
            locale: Locale::It,
            text: None,
            attachments: files
                .iter()
                .enumerate()
                .map(|(i, (kind, data))| AttachmentMeta {
                    id: i as u16,
                    kind: *kind,
                    mime: "x/y".into(),
                    size: data.len() as u64,
                    sha256: sha256(data),
                    duration_ms: None,
                })
                .collect(),
        }
    }

    fn receiver(p: LinkProfile) -> NoteReceiver {
        let mut r = NoteReceiver::new(RetryPolicy::default(), vec![]);
        r.link_up(p);
        r
    }

    fn offer(r: &mut NoteReceiver, m: &NoteManifest, now: u64) -> Vec<ReceiverEffect> {
        r.on_frame(
            Frame::NoteOffer(NoteOffer {
                manifest: m.clone(),
            }),
            now,
        )
    }

    fn chunk(att: u16, offset: u64, data: &[u8]) -> Frame {
        Frame::NoteChunk(NoteChunk {
            note_id: NOTE,
            attachment_id: att,
            offset,
            data: data.to_vec(),
            sha256: sha256(data),
        })
    }

    fn requests(e: &[ReceiverEffect]) -> Vec<(u16, u64)> {
        e.iter()
            .filter_map(|e| match e {
                ReceiverEffect::Send(Frame::NoteChunkRequest(r)) => {
                    Some((r.attachment_id, r.offset))
                }
                _ => None,
            })
            .collect()
    }

    fn writes(e: &[ReceiverEffect]) -> Vec<u64> {
        e.iter()
            .filter_map(|e| match e {
                ReceiverEffect::WriteChunk { offset, .. } => Some(*offset),
                _ => None,
            })
            .collect()
    }

    fn status(e: &[ReceiverEffect]) -> Option<&NoteOfferStatus> {
        e.iter().find_map(|e| match e {
            ReceiverEffect::Send(Frame::NoteOfferReply(r)) => Some(&r.status),
            _ => None,
        })
    }

    fn position(e: &[ReceiverEffect], f: impl Fn(&ReceiverEffect) -> bool) -> usize {
        e.iter().position(f).unwrap()
    }

    #[test]
    fn accepted_is_durable_first_and_the_pull_respects_the_window() {
        let mut r = receiver(profile(4, 2, None));
        let m = manifest(&[(AttachmentKind::Photo, &[0; 10])]);
        let e = offer(&mut r, &m, 0);
        let persist = position(&e, |e| matches!(e, ReceiverEffect::Persist(_)));
        let reply = position(&e, |e| {
            matches!(e, ReceiverEffect::Send(Frame::NoteOfferReply(_)))
        });
        assert!(persist < reply);
        assert_eq!(requests(&e), vec![(0, 0), (0, 4)], "two in flight");

        let e = r.on_frame(chunk(0, 0, &[0; 4]), 1);
        assert_eq!(writes(&e), vec![0]);
        assert_eq!(requests(&e), vec![(0, 8)], "one out, one in");
    }

    #[test]
    fn voice_first_then_small_to_large() {
        let mut r = receiver(profile(64, 1, None));
        let m = manifest(&[
            (AttachmentKind::Photo, &[0; 30]),
            (AttachmentKind::Photo, &[0; 10]),
            (AttachmentKind::VoiceRecording, &[0; 50]),
        ]);
        assert_eq!(requests(&offer(&mut r, &m, 0)), vec![(2, 0)]);
    }

    #[test]
    fn corrupt_chunk_is_rejected_unwritten_and_fetched_again() {
        let mut r = receiver(profile(4, 2, None));
        let m = manifest(&[(AttachmentKind::Photo, &[0; 12])]);
        offer(&mut r, &m, 0);
        let bad = Frame::NoteChunk(NoteChunk {
            note_id: NOTE,
            attachment_id: 0,
            offset: 0,
            data: vec![1; 4], // Damaged on the way: the digest is of the original.
            sha256: sha256(&[0; 4]),
        });
        let e = r.on_frame(bad, 1);
        assert!(writes(&e).is_empty());
        assert!(e.contains(&ReceiverEffect::Send(Frame::NoteChunkReject(
            NoteChunkReject {
                note_id: NOTE,
                attachment_id: 0,
                offset: 0,
                reason: NoteChunkRejectReason::Corrupt,
            }
        ))));
        assert_eq!(requests(&e), vec![(0, 0), (0, 4)], "the window restarts");

        // The rest of the old window drains without a second rejection.
        let e = r.on_frame(chunk(0, 4, &[0; 4]), 2);
        assert!(e.is_empty(), "{e:?}");

        // Then the good copies are taken, and a stale repeat is harmless.
        assert_eq!(writes(&r.on_frame(chunk(0, 0, &[0; 4]), 3)), vec![0]);
        assert!(r.on_frame(chunk(0, 0, &[0; 4]), 4).is_empty());
        assert_eq!(writes(&r.on_frame(chunk(0, 4, &[0; 4]), 5)), vec![4]);
    }

    #[test]
    fn wrong_length_and_skipped_chunks_are_rejected() {
        let mut r = receiver(profile(4, 1, None));
        let m = manifest(&[(AttachmentKind::Photo, &[0; 12])]);
        offer(&mut r, &m, 0);
        let e = r.on_frame(chunk(0, 0, &[0; 3]), 1);
        assert!(e.iter().any(|e| matches!(
            e,
            ReceiverEffect::Send(Frame::NoteChunkReject(NoteChunkReject {
                reason: NoteChunkRejectReason::BadLength,
                ..
            }))
        )));
        r.on_frame(chunk(0, 0, &[0; 4]), 2);
        // Now not rewinding: a jump ahead means bytes went missing.
        let e = r.on_frame(chunk(0, 8, &[0; 4]), 3);
        assert!(writes(&e).is_empty());
        assert!(e.iter().any(|e| matches!(
            e,
            ReceiverEffect::Send(Frame::NoteChunkReject(NoteChunkReject {
                reason: NoteChunkRejectReason::Gap,
                ..
            }))
        )));
        assert_eq!(requests(&e), vec![(0, 4)]);
    }

    #[test]
    fn endless_corruption_pauses_the_transfer_instead_of_looping() {
        let mut r = receiver(profile(4, 1, None));
        let m = manifest(&[(AttachmentKind::Photo, &[0; 12])]);
        offer(&mut r, &m, 0);
        let bad = Frame::NoteChunk(NoteChunk {
            note_id: NOTE,
            attachment_id: 0,
            offset: 0,
            data: vec![1; 4],
            sha256: [0; 32],
        });
        let mut last = Vec::new();
        for i in 0..MAX_CONSECUTIVE_REJECTS {
            last = r.on_frame(bad.clone(), u64::from(i));
        }
        assert_eq!(
            status(&last),
            Some(&NoteOfferStatus::Deferred {
                reason: NoteDeferReason::IntegrityFailure
            })
        );
        assert!(r.next_deadline().is_none());
    }

    #[test]
    fn an_offer_of_another_note_means_the_glass_moved_on() {
        let mut r = receiver(profile(4, 1, None));
        let first = manifest(&[(AttachmentKind::Photo, &[0; 12])]);
        offer(&mut r, &first, 0);
        r.on_frame(chunk(0, 0, &[0; 4]), 1);

        let mut second = manifest(&[(AttachmentKind::Photo, &[1; 8])]);
        second.id = NoteId([2; 16]);
        let e = offer(&mut r, &second, 2);
        assert!(
            e.iter()
                .any(|e| matches!(e, ReceiverEffect::SyncPartial { .. })),
            "the first note keeps what it had"
        );
        assert_eq!(r.note(&NOTE).unwrap().attachments[&0].committed_offset, 4);
        assert!(matches!(status(&e), Some(NoteOfferStatus::Accepted { .. })));
        // Late chunks of the first note go nowhere.
        assert!(r.on_frame(chunk(0, 4, &[0; 4]), 3).is_empty());
    }

    #[test]
    fn duplicate_offer_changes_nothing_mid_transfer() {
        let mut r = receiver(profile(4, 1, None));
        let m = manifest(&[(AttachmentKind::Photo, &[0; 12])]);
        offer(&mut r, &m, 0);
        r.on_frame(chunk(0, 0, &[0; 4]), 1);
        let e = offer(&mut r, &m, 2);
        assert_eq!(e.len(), 1, "only an answer: {e:?}");
        match status(&e) {
            Some(NoteOfferStatus::Accepted { progress }) => {
                assert_eq!(progress[0].next_offset, 4)
            }
            other => panic!("{other:?}"),
        }
        // And the transfer simply carries on.
        assert_eq!(writes(&r.on_frame(chunk(0, 4, &[0; 4]), 3)), vec![4]);
    }

    fn finish_all(r: &mut NoteReceiver, data: &[u8]) -> Vec<ReceiverEffect> {
        let e = r.on_frame(chunk(0, 0, data), 1);
        assert!(e
            .iter()
            .any(|e| matches!(e, ReceiverEffect::VerifyAttachment { .. })));
        r.on_attachment_hashed(NOTE, 0, sha256(data), 2)
    }

    #[test]
    fn ack_goes_out_only_after_commit_and_persist() {
        let mut r = receiver(profile(64, 1, None));
        let m = manifest(&[(AttachmentKind::Photo, b"data")]);
        offer(&mut r, &m, 0);
        let e = finish_all(&mut r, b"data");
        let commit = position(&e, |e| matches!(e, ReceiverEffect::CommitAttachment { .. }));
        let persist = position(&e, |e| matches!(e, ReceiverEffect::Persist(_)));
        let att_ack = position(&e, |e| {
            matches!(e, ReceiverEffect::Send(Frame::NoteAttachmentAck(_)))
        });
        let note_ack = position(&e, |e| matches!(e, ReceiverEffect::Send(Frame::NoteAck(_))));
        assert!(commit < persist && persist < att_ack && att_ack < note_ack);
        assert!(r.note(&NOTE).unwrap().complete);
    }

    #[test]
    fn repeat_offer_after_completion_reacknowledges_for_a_lost_ack() {
        let mut r = receiver(profile(64, 1, None));
        let m = manifest(&[(AttachmentKind::Photo, b"data")]);
        offer(&mut r, &m, 0);
        finish_all(&mut r, b"data");
        let e = offer(&mut r, &m, 3);
        assert!(requests(&e).is_empty());
        let ack = position(&e, |e| {
            matches!(
                e,
                ReceiverEffect::Send(Frame::NoteAttachmentAck(NoteAttachmentAck {
                    verified: true,
                    ..
                }))
            )
        });
        let reply = position(&e, |e| {
            matches!(e, ReceiverEffect::Send(Frame::NoteOfferReply(_)))
        });
        assert!(ack < reply);
        assert!(matches!(
            status(&e),
            Some(NoteOfferStatus::AlreadyComplete { .. })
        ));
    }

    #[test]
    fn same_id_with_a_different_manifest_is_a_conflict_and_invalid_is_refused() {
        let mut r = receiver(profile(64, 1, None));
        offer(&mut r, &manifest(&[(AttachmentKind::Photo, b"data")]), 0);
        let e = offer(&mut r, &manifest(&[(AttachmentKind::Photo, b"other")]), 1);
        assert_eq!(
            status(&e),
            Some(&NoteOfferStatus::Rejected {
                reason: NoteRejectReason::ManifestConflict
            })
        );
        let mut invalid = manifest(&[(AttachmentKind::Photo, b"data")]);
        invalid.id = NoteId([2; 16]);
        invalid.attachments[0].size = u64::MAX;
        let e = offer(&mut r, &invalid, 2);
        assert_eq!(
            status(&e),
            Some(&NoteOfferStatus::Rejected {
                reason: NoteRejectReason::InvalidManifest
            })
        );
        assert!(r.note(&invalid.id).is_none(), "nothing stored for it");
    }

    #[test]
    fn whole_file_mismatch_restarts_then_pauses_without_acknowledging() {
        let mut r = receiver(profile(64, 1, None));
        let m = manifest(&[(AttachmentKind::Photo, b"data")]);
        offer(&mut r, &m, 0);
        for round in 0..MAX_HASH_FAILURES {
            let e = r.on_frame(chunk(0, 0, b"data"), 1);
            assert!(e
                .iter()
                .any(|e| matches!(e, ReceiverEffect::VerifyAttachment { .. })));
            let e = r.on_attachment_hashed(NOTE, 0, [0xee; 32], 2);
            assert!(e
                .iter()
                .any(|e| matches!(e, ReceiverEffect::TruncatePartial { len: 0, .. })));
            assert!(e.iter().any(|e| matches!(
                e,
                ReceiverEffect::Send(Frame::NoteAttachmentAck(NoteAttachmentAck {
                    verified: false,
                    ..
                }))
            )));
            if round + 1 < MAX_HASH_FAILURES {
                assert_eq!(requests(&e), vec![(0, 0)]);
            } else {
                assert_eq!(
                    status(&e),
                    Some(&NoteOfferStatus::Deferred {
                        reason: NoteDeferReason::IntegrityFailure
                    })
                );
            }
        }
        assert!(!r.note(&NOTE).unwrap().complete);
    }

    #[test]
    fn link_drop_checkpoints_and_resume_continues_from_there() {
        let mut r = receiver(profile(4, 1, None));
        let m = manifest(&[(AttachmentKind::Photo, &[0; 12])]);
        offer(&mut r, &m, 0);
        r.on_frame(chunk(0, 0, &[0; 4]), 1);
        let e = r.link_down();
        let sync = position(&e, |e| matches!(e, ReceiverEffect::SyncPartial { .. }));
        let persist = position(&e, |e| matches!(e, ReceiverEffect::Persist(_)));
        assert!(sync < persist, "data before the record that claims it");
        assert_eq!(r.note(&NOTE).unwrap().attachments[&0].committed_offset, 4);

        r.link_up(profile(4, 1, None));
        let e = offer(&mut r, &m, 2);
        assert!(e.contains(&ReceiverEffect::TruncatePartial {
            note_id: NOTE,
            attachment_id: 0,
            len: 4
        }));
        assert_eq!(requests(&e), vec![(0, 4)]);
    }

    #[test]
    fn resume_after_a_crash_falls_back_to_the_last_checkpoint() {
        let m = manifest(&[(AttachmentKind::Photo, &[0; 12])]);
        // What the record says after a crash: 4 bytes durable, whatever the file holds.
        let record = IncomingNote {
            format: RECORD_FORMAT,
            manifest_sha256: manifest_digest(&m),
            attachments: BTreeMap::from([(
                0,
                IncomingAttachment {
                    committed_offset: 4,
                    verified: false,
                    hash_failures: 0,
                },
            )]),
            manifest: m.clone(),
            complete: false,
            transcripts: BTreeMap::new(),
            received_at_ms: 0,
        };
        let mut r = NoteReceiver::new(RetryPolicy::default(), vec![record]);
        r.link_up(profile(4, 1, None));
        let e = offer(&mut r, &m, 0);
        assert!(e.contains(&ReceiverEffect::TruncatePartial {
            note_id: NOTE,
            attachment_id: 0,
            len: 4
        }));
    }

    #[test]
    fn link_drop_after_the_last_byte_resumes_with_the_check() {
        let mut r = receiver(profile(4, 1, None));
        let m = manifest(&[(AttachmentKind::Photo, &[0; 4])]);
        offer(&mut r, &m, 0);
        let e = r.on_frame(chunk(0, 0, &[0; 4]), 1);
        assert!(e
            .iter()
            .any(|e| matches!(e, ReceiverEffect::VerifyAttachment { .. })));
        // Suppose the driver did not get to answer before the link went.
        r.link_down();
        r.link_up(profile(4, 1, None));
        let e = offer(&mut r, &m, 2);
        assert!(requests(&e).is_empty());
        assert!(e
            .iter()
            .any(|e| matches!(e, ReceiverEffect::VerifyAttachment { .. })));
    }

    #[test]
    fn silence_asks_again_then_gives_up_until_the_next_offer() {
        let mut r = receiver(profile(4, 1, None));
        let m = manifest(&[(AttachmentKind::Photo, &[0; 12])]);
        offer(&mut r, &m, 0);
        let mut now = 0;
        for _ in 0..MAX_STALLS {
            now = r.next_deadline().unwrap();
            assert!(r.tick(now - 1).is_empty());
            assert_eq!(requests(&r.tick(now)), vec![(0, 0)]);
        }
        now += RetryPolicy::default().response_timeout_ms;
        assert!(requests(&r.tick(now)).is_empty());
        assert!(r.next_deadline().is_none());
        assert_eq!(requests(&offer(&mut r, &m, now)), vec![(0, 0)]);
    }

    #[test]
    fn too_big_for_this_link_waits_for_a_better_one_after_the_rest_is_in() {
        let mut r = receiver(profile(64, 1, Some(8)));
        let m = manifest(&[
            (AttachmentKind::VoiceRecording, b"voice"),
            (AttachmentKind::Video, &[0; 20]),
        ]);
        assert_eq!(requests(&offer(&mut r, &m, 0)), vec![(0, 0)]);
        r.on_frame(chunk(0, 0, b"voice"), 1);
        let e = r.on_attachment_hashed(NOTE, 0, sha256(b"voice"), 2);
        assert_eq!(
            status(&e),
            Some(&NoteOfferStatus::Deferred {
                reason: NoteDeferReason::NeedsFasterLink { bytes: 20 }
            })
        );
        assert!(!r.note(&NOTE).unwrap().complete);
        // The same note on a fast link picks up the video, without touching the voice.
        r.link_down();
        r.link_up(profile(64, 1, None));
        assert_eq!(requests(&offer(&mut r, &m, 3)), vec![(1, 0)]);
    }

    #[test]
    fn known_short_space_defers_and_a_write_failure_pauses() {
        let mut r = receiver(profile(4, 1, None));
        let m = manifest(&[(AttachmentKind::Photo, &[0; 12])]);
        r.set_free_bytes(Some(11));
        let e = offer(&mut r, &m, 0);
        assert!(matches!(
            status(&e),
            Some(NoteOfferStatus::Deferred {
                reason: NoteDeferReason::InsufficientStorage
            })
        ));
        r.set_free_bytes(Some(12));
        assert!(!requests(&offer(&mut r, &m, 1)).is_empty());
        let e = r.on_storage_error(NOTE);
        assert!(matches!(
            status(&e),
            Some(NoteOfferStatus::Deferred {
                reason: NoteDeferReason::InsufficientStorage
            })
        ));
    }

    fn voice_note_received() -> (NoteReceiver, NoteManifest) {
        let mut r = receiver(profile(64, 1, None));
        let m = manifest(&[(AttachmentKind::VoiceRecording, b"voice")]);
        offer(&mut r, &m, 0);
        finish_all(&mut r, b"voice");
        (r, m)
    }

    fn ask_transcript(r: &mut NoteReceiver) -> Vec<ReceiverEffect> {
        r.on_frame(
            Frame::NoteTranscriptRequest(NoteTranscriptRequest {
                note_id: NOTE,
                attachment_id: 0,
            }),
            5,
        )
    }

    #[test]
    fn transcription_is_idempotent_and_survives_until_acknowledged() {
        let (mut r, _) = voice_note_received();
        let e = ask_transcript(&mut r);
        assert_eq!(
            e.iter()
                .filter(|e| matches!(e, ReceiverEffect::Transcribe { .. }))
                .count(),
            1
        );
        assert!(ask_transcript(&mut r).is_empty(), "running already");
        assert!(!r.is_idle());

        let e = r.on_transcribed(NOTE, 0, Ok("ciao".into()));
        let persist = position(&e, |e| matches!(e, ReceiverEffect::Persist(_)));
        let send = position(&e, |e| {
            matches!(e, ReceiverEffect::Send(Frame::NoteTranscript(_)))
        });
        assert!(persist < send);

        // A repeat gets the stored text, with no second engine run.
        let e = ask_transcript(&mut r);
        assert!(matches!(
            e[..],
            [ReceiverEffect::Send(Frame::NoteTranscript(_))]
        ));

        // Not acknowledged: it goes out again on the next link.
        r.link_down();
        let e = r.link_up(profile(64, 1, None));
        assert!(matches!(
            e[..],
            [ReceiverEffect::Send(Frame::NoteTranscript(_))]
        ));

        r.on_frame(
            Frame::NoteTranscriptAck(NoteTranscriptAck {
                note_id: NOTE,
                attachment_id: 0,
            }),
            6,
        );
        r.link_down();
        assert!(r.link_up(profile(64, 1, None)).is_empty());
    }

    #[test]
    fn transcription_interrupted_by_a_restart_runs_again() {
        let (mut r, _) = voice_note_received();
        ask_transcript(&mut r);
        let record = r.note(&NOTE).unwrap().clone();
        let mut restarted = NoteReceiver::new(RetryPolicy::default(), vec![record]);
        let e = restarted.link_up(profile(64, 1, None));
        assert!(matches!(e[..], [ReceiverEffect::Transcribe { .. }]));
    }

    #[test]
    fn engine_failure_is_reported_and_a_new_request_retries() {
        let (mut r, _) = voice_note_received();
        ask_transcript(&mut r);
        let e = r.on_transcribed(NOTE, 0, Err(SpeechError::Unavailable));
        assert!(e.iter().any(|e| matches!(
            e,
            ReceiverEffect::Send(Frame::NoteTranscript(NoteTranscript {
                error: Some(NoteTranscriptError::EngineUnavailable),
                ..
            }))
        )));
        assert!(ask_transcript(&mut r)
            .iter()
            .any(|e| matches!(e, ReceiverEffect::Transcribe { .. })));
    }

    #[test]
    fn transcript_needs_the_verified_recording_and_a_recording_at_all() {
        let mut r = receiver(profile(64, 1, None));
        assert!(ask_transcript(&mut r).iter().any(|e| matches!(
            e,
            ReceiverEffect::Send(Frame::NoteTranscript(NoteTranscript {
                error: Some(NoteTranscriptError::AudioNotAvailable),
                ..
            }))
        )));
        offer(
            &mut r,
            &manifest(&[(AttachmentKind::VoiceRecording, b"voice")]),
            0,
        );
        assert!(ask_transcript(&mut r).iter().any(|e| matches!(
            e,
            ReceiverEffect::Send(Frame::NoteTranscript(NoteTranscript {
                error: Some(NoteTranscriptError::AudioNotAvailable),
                ..
            }))
        )));

        let mut r = receiver(profile(64, 1, None));
        offer(&mut r, &manifest(&[(AttachmentKind::Photo, b"p")]), 0);
        assert!(ask_transcript(&mut r).iter().any(|e| matches!(
            e,
            ReceiverEffect::Send(Frame::NoteTranscript(NoteTranscript {
                error: Some(NoteTranscriptError::NotAVoiceRecording),
                ..
            }))
        )));
    }

    #[test]
    fn long_transcript_is_cut_on_a_character_boundary_and_kept_whole_here() {
        let (mut r, _) = voice_note_received();
        ask_transcript(&mut r);
        let long = "é".repeat(MAX_TRANSCRIPT_BYTES); // Twice the limit, in bytes.
        let e = r.on_transcribed(NOTE, 0, Ok(long.clone()));
        let sent = e
            .iter()
            .find_map(|e| match e {
                ReceiverEffect::Send(Frame::NoteTranscript(t)) => Some(t),
                _ => None,
            })
            .unwrap();
        assert!(sent.truncated);
        assert!(sent.text.len() <= MAX_TRANSCRIPT_BYTES);
        let kept = r.note(&NOTE).unwrap().transcripts[&0].result.as_ref();
        assert_eq!(kept.unwrap().text, long);
    }

    #[test]
    fn tts_is_served_in_pieces_bounded_and_freed_on_release() {
        let mut r = receiver(profile(64, 1, None));
        let ask = |text: Option<&str>, request_id| {
            Frame::NoteTtsRequest(NoteTtsRequest {
                request_id,
                note_id: NOTE,
                text: text.map(String::from),
                locale: Locale::It,
            })
        };
        // Nothing to say for an unknown note, and no text.
        let e = r.on_frame(ask(None, 1), 0);
        assert!(matches!(
            e[..],
            [ReceiverEffect::Send(Frame::NoteTtsFailed(_))]
        ));
        let too_long = "x".repeat(MAX_TTS_TEXT_BYTES + 1);
        let e = r.on_frame(ask(Some(&too_long), 2), 0);
        assert!(matches!(
            e[..],
            [ReceiverEffect::Send(Frame::NoteTtsFailed(_))]
        ));

        let e = r.on_frame(ask(Some("ciao"), 3), 0);
        assert!(matches!(e[..], [ReceiverEffect::Synthesize { .. }]));
        assert!(r.on_frame(ask(Some("ciao"), 3), 0).is_empty(), "running");

        let e = r.on_synthesized(
            3,
            NOTE,
            Ok(SynthesizedAudio {
                sample_rate: 16_000,
                pcm: vec![1, 2, 3, 4, 5],
            }),
        );
        assert!(matches!(
            e[..],
            [ReceiverEffect::Send(Frame::NoteTtsReady(_))]
        ));
        let pull = |offset, max_len| {
            Frame::NoteTtsChunkRequest(NoteTtsChunkRequest {
                request_id: 3,
                offset,
                max_len,
            })
        };
        let e = r.on_frame(pull(1, 2), 1);
        match &e[..] {
            [ReceiverEffect::Send(Frame::NoteTtsChunk(c))] => {
                assert_eq!(c.data, vec![2, 3]);
                assert_eq!(c.sha256, sha256(&[2, 3]));
            }
            other => panic!("{other:?}"),
        }
        assert!(r.on_frame(pull(5, 2), 1).is_empty(), "past the end");
        assert!(r.on_frame(pull(u64::MAX, 2), 1).is_empty());

        r.on_frame(Frame::NoteTtsRelease(NoteTtsRelease { request_id: 3 }), 2);
        assert!(r.on_frame(pull(0, 2), 3).is_empty());
    }
}
