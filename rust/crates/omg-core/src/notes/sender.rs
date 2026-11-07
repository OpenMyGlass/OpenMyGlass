//! The Glass side of note sync: a pure state machine.
//!
//! It holds the queue, offers one note at a time, serves the chunks the companion asks
//! for, and decides when a note is really safe. It touches nothing itself: every call
//! returns [`SenderEffect`]s for the caller to carry out, in order. Any effect that fails
//! must stop the ones after it.
//!
//! Local files go away only when the delete policy asks for it **and** the companion has
//! acknowledged every attachment and the whole manifest with digests matching this side's
//! own record. Any other acknowledgement is ignored.

use std::collections::{BTreeMap, BTreeSet};

use omg_protocol::{
    AttachmentId, AttachmentKind, Frame, Locale, NoteAttachmentAck, NoteChunk, NoteChunkRequest,
    NoteChunkUnavailable, NoteDeferReason, NoteId, NoteOffer, NoteOfferReply, NoteOfferStatus,
    NoteSyncStatus, NoteTranscript, NoteTranscriptAck, NoteTranscriptError, NoteTranscriptRequest,
    NoteTtsChunk, NoteTtsChunkRequest, NoteTtsReady, NoteTtsRelease, NoteTtsRequest,
    NoteUnavailableReason, Sha256Digest, MAX_NOTE_CHUNK_LEN, MAX_TTS_AUDIO_BYTES,
};
use sha2::{Digest, Sha256};

use super::{
    record::{BlockReason, OutgoingNote, OutgoingState, ReceivedTranscript},
    sha256, LinkProfile, RetryPolicy,
};

/// Bad audio chunks or silent timeouts tolerated before a TTS pull gives up.
const MAX_TTS_RETRIES: u32 = 4;

/// What to do with a note's files on the Glass once the companion has verified it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DeletePolicy {
    /// Keep them.
    Keep,
    /// Delete them after full verification. Applies to notes verified while it is set.
    /// Owner default, still open: see `docs/companion-notes-sync.md`.
    #[default]
    DeleteAfterVerified,
}

/// Glass-side settings.
#[derive(Debug, Clone, Copy, Default)]
pub struct SenderConfig {
    /// After full verification.
    pub delete_policy: DeletePolicy,
    /// Retry timing.
    pub retry: RetryPolicy,
    /// Do not ask the companion to transcribe voice recordings.
    pub no_auto_transcribe: bool,
}

/// Something that happened, for the Glass UI.
#[derive(Debug, Clone, PartialEq)]
pub enum SenderEvent {
    /// The companion verified the whole note.
    NoteSynced {
        /// The note.
        note_id: NoteId,
    },
    /// The companion paused this note. It stays queued.
    NoteDeferred {
        /// The note.
        note_id: NoteId,
        /// Why.
        reason: NoteDeferReason,
    },
    /// This note will not be offered again until retried. It is kept.
    NoteBlocked {
        /// The note.
        note_id: NoteId,
        /// Why.
        reason: BlockReason,
    },
    /// An acknowledgement did not match this side's record, or came too early. It changed
    /// nothing.
    AckIgnored {
        /// The note.
        note_id: NoteId,
    },
    /// A transcript arrived.
    Transcript {
        /// The note.
        note_id: NoteId,
        /// The voice attachment.
        attachment_id: AttachmentId,
        /// What it says.
        transcript: ReceivedTranscript,
    },
    /// Speech to play. Pieces arrive in order.
    TtsAudio {
        /// The request.
        request_id: u32,
        /// The note.
        note_id: NoteId,
        /// PCM sample rate.
        sample_rate: u32,
        /// PCM 16-bit little-endian mono.
        data: Vec<u8>,
    },
    /// A TTS request ended.
    TtsFinished {
        /// The request.
        request_id: u32,
        /// The note.
        note_id: NoteId,
        /// Set if it failed.
        error: Option<String>,
    },
}

/// Something the caller must do, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum SenderEffect {
    /// Send this frame to the companion.
    Send(Frame),
    /// Read these bytes and call [`NoteSender::on_chunk_read`].
    ReadChunk {
        /// The note.
        note_id: NoteId,
        /// The attachment.
        attachment_id: AttachmentId,
        /// Where to read.
        offset: u64,
        /// How much; always within the file.
        len: u32,
    },
    /// Hash the local file and call [`NoteSender::on_source_checked`].
    VerifySource {
        /// The note.
        note_id: NoteId,
        /// The attachment.
        attachment_id: AttachmentId,
    },
    /// Store this record durably.
    Persist(Box<OutgoingNote>),
    /// Remove the note's files and record. The record already says this is due.
    DeleteLocal {
        /// The note.
        note_id: NoteId,
    },
    /// Tell the UI.
    Event(SenderEvent),
}

struct Entry {
    record: OutgoingNote,
    attempts: u32,
    not_before: u64,
}

struct Active {
    note_id: NoteId,
    deadline: u64,
}

struct TtsPull {
    request_id: u32,
    note_id: NoteId,
    ready: Option<TtsStream>,
    retries: u32,
    deadline: u64,
}

struct TtsStream {
    sample_rate: u32,
    total: u64,
    sha256: Sha256Digest,
    next_offset: u64,
    hasher: Sha256,
}

/// Counts for a UI, and the same numbers the companion gets in `NoteSyncStatus`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueStats {
    /// Notes not yet fully verified on the companion.
    pub pending_notes: u32,
    /// Bytes of their attachments not yet verified.
    pub pending_bytes: u64,
}

/// See the module docs.
pub struct NoteSender {
    cfg: SenderConfig,
    notes: BTreeMap<NoteId, Entry>,
    link: Option<LinkProfile>,
    active: Option<Active>,
    transcript_requested: BTreeSet<(NoteId, AttachmentId)>,
    tts: Option<TtsPull>,
    next_tts_id: u32,
    last_status: Option<NoteSyncStatus>,
}

impl NoteSender {
    /// Rebuild from the persisted queue.
    pub fn new(cfg: SenderConfig, records: Vec<OutgoingNote>) -> Self {
        let notes = records
            .into_iter()
            .map(|record| {
                let entry = Entry {
                    record,
                    attempts: 0,
                    not_before: 0,
                };
                (entry.record.manifest.id, entry)
            })
            .collect();
        NoteSender {
            cfg,
            notes,
            link: None,
            active: None,
            transcript_requested: BTreeSet::new(),
            tts: None,
            next_tts_id: 1,
            last_status: None,
        }
    }

    /// Finish what a crash interrupted: deletes that were recorded as due. Call once
    /// after [`new`](NoteSender::new).
    pub fn startup(&mut self) -> Vec<SenderEffect> {
        let due: Vec<NoteId> = self
            .notes
            .values()
            .filter(|e| {
                matches!(
                    e.record.state,
                    OutgoingState::Synced {
                        delete_pending: true
                    }
                )
            })
            .map(|e| e.record.manifest.id)
            .collect();
        due.into_iter()
            .map(|note_id| {
                self.notes.remove(&note_id);
                SenderEffect::DeleteLocal { note_id }
            })
            .collect()
    }

    /// The queue position for the next sealed note.
    pub fn next_seq(&self) -> u64 {
        self.notes
            .values()
            .map(|e| e.record.seq + 1)
            .max()
            .unwrap_or(0)
    }

    /// Add a note already sealed in the store. If a link is up it syncs right away;
    /// otherwise it waits for one.
    pub fn enqueue(&mut self, record: OutgoingNote, now: u64) -> Vec<SenderEffect> {
        let mut out = Vec::new();
        self.notes.insert(
            record.manifest.id,
            Entry {
                record,
                attempts: 0,
                not_before: 0,
            },
        );
        self.advance(now, &mut out);
        out
    }

    /// The owner deleted a note from this queue. `None` refuses: it is unknown, or it is being
    /// sent right now.
    pub fn forget(&mut self, note_id: &NoteId) -> Option<Vec<SenderEffect>> {
        if self.active.as_ref().is_some_and(|a| a.note_id == *note_id) {
            return None;
        }
        self.notes.remove(note_id)?;
        Some(vec![SenderEffect::DeleteLocal { note_id: *note_id }])
    }

    /// A record in the queue.
    pub fn note(&self, id: &NoteId) -> Option<&OutgoingNote> {
        self.notes.get(id).map(|e| &e.record)
    }

    /// All records in queue order.
    pub fn notes(&self) -> Vec<&OutgoingNote> {
        let mut all: Vec<_> = self.notes.values().map(|e| &e.record).collect();
        all.sort_by_key(|r| r.seq);
        all
    }

    /// What is still waiting.
    pub fn stats(&self) -> QueueStats {
        let mut stats = QueueStats::default();
        for entry in self.notes.values() {
            let record = &entry.record;
            if matches!(record.state, OutgoingState::Synced { .. }) {
                continue;
            }
            stats.pending_notes += 1;
            stats.pending_bytes += record
                .manifest
                .attachments
                .iter()
                .filter(|a| !record.verified.contains(&a.id))
                .map(|a| a.size)
                .sum::<u64>();
        }
        stats
    }

    /// The earliest time [`tick`](NoteSender::tick) has something to do.
    pub fn next_deadline(&self) -> Option<u64> {
        self.link?;
        let active = self.active.as_ref().map(|a| a.deadline);
        let tts = self.tts.as_ref().map(|t| t.deadline);
        let parked = if self.active.is_none() {
            self.notes
                .values()
                .filter(|e| e.record.state == OutgoingState::Queued)
                .map(|e| e.not_before)
                .min()
        } else {
            None
        };
        [active, tts, parked].into_iter().flatten().min()
    }

    /// A link is up and authenticated, and the peer advertised the notes capability.
    /// Delays from the last link are forgotten: a better link may have changed the answer.
    pub fn link_up(&mut self, profile: LinkProfile, now: u64) -> Vec<SenderEffect> {
        self.link = Some(profile.normalized());
        self.active = None;
        self.tts = None;
        self.last_status = None;
        self.transcript_requested.clear();
        for entry in self.notes.values_mut() {
            entry.attempts = 0;
            entry.not_before = 0;
        }
        let mut out = Vec::new();
        let ids: Vec<NoteId> = self.notes.keys().copied().collect();
        for id in ids {
            self.request_transcripts(&id, &mut out);
        }
        self.advance(now, &mut out);
        out
    }

    /// The link went away. Nothing about the queue changes.
    pub fn link_down(&mut self) {
        self.link = None;
        self.active = None;
        self.tts = None;
    }

    /// Time passed.
    pub fn tick(&mut self, now: u64) -> Vec<SenderEffect> {
        let mut out = Vec::new();
        if self.link.is_none() {
            return out;
        }
        if let Some(tts) = &mut self.tts {
            if now >= tts.deadline {
                tts.retries += 1;
                let (request_id, retries) = (tts.request_id, tts.retries);
                if retries > MAX_TTS_RETRIES {
                    self.finish_tts(Some("timed out".into()), &mut out);
                } else {
                    let resend = self.tts_chunk_request(request_id);
                    if let Some(tts) = &mut self.tts {
                        tts.deadline = now + self.cfg.retry.response_timeout_ms;
                    }
                    out.extend(resend.map(SenderEffect::Send));
                }
            }
        }
        if let Some(active) = &self.active {
            if now >= active.deadline {
                let note_id = active.note_id;
                let retry = self.cfg.retry;
                let attempts = self.notes.get_mut(&note_id).map(|entry| {
                    entry.attempts += 1;
                    out.push(SenderEffect::Send(Frame::NoteOffer(NoteOffer {
                        manifest: entry.record.manifest.clone(),
                    })));
                    entry.attempts
                });
                match attempts {
                    Some(n) => {
                        self.active = Some(Active {
                            note_id,
                            deadline: now + retry.response_timeout_ms + retry.backoff(n),
                        })
                    }
                    None => self.active = None,
                }
            }
        }
        self.advance(now, &mut out);
        out
    }

    /// A frame arrived. Frames that are not this side's are ignored.
    pub fn on_frame(&mut self, frame: Frame, now: u64) -> Vec<SenderEffect> {
        let mut out = Vec::new();
        match frame {
            Frame::NoteOfferReply(reply) => self.on_offer_reply(reply, now, &mut out),
            Frame::NoteChunkRequest(request) => self.on_chunk_request(request, now, &mut out),
            Frame::NoteChunkReject(reject) => self.refresh(&reject.note_id, now),
            Frame::NoteChunkUnavailable(_) => {}
            Frame::NoteAttachmentAck(ack) => self.on_attachment_ack(ack, now, &mut out),
            Frame::NoteAck(ack) => {
                self.try_mark_synced(&ack.note_id, &ack.manifest_sha256, now, &mut out)
            }
            Frame::NoteTranscript(t) => self.on_transcript(t, &mut out),
            Frame::NoteTtsReady(ready) => self.on_tts_ready(ready, now, &mut out),
            Frame::NoteTtsFailed(failed) => {
                if self.tts.as_ref().map(|t| t.request_id) == Some(failed.request_id) {
                    self.finish_tts(Some(failed.reason), &mut out);
                }
            }
            Frame::NoteTtsChunk(chunk) => self.on_tts_chunk(chunk, now, &mut out),
            _ => {}
        }
        out
    }

    /// Answer to [`SenderEffect::ReadChunk`].
    pub fn on_chunk_read(
        &mut self,
        note_id: NoteId,
        attachment_id: AttachmentId,
        offset: u64,
        data: Result<Vec<u8>, ()>,
    ) -> Vec<SenderEffect> {
        match data {
            Ok(data) => {
                let sha256 = sha256(&data);
                vec![SenderEffect::Send(Frame::NoteChunk(NoteChunk {
                    note_id,
                    attachment_id,
                    offset,
                    data,
                    sha256,
                }))]
            }
            Err(()) => vec![
                SenderEffect::Send(Frame::NoteChunkUnavailable(NoteChunkUnavailable {
                    note_id,
                    attachment_id,
                    offset,
                    reason: NoteUnavailableReason::SourceError,
                })),
                SenderEffect::VerifySource {
                    note_id,
                    attachment_id,
                },
            ],
        }
    }

    /// Answer to [`SenderEffect::VerifySource`]: the file's digest, or `None` if it could
    /// not be read. A file that no longer matches is kept and blocks the note.
    pub fn on_source_checked(
        &mut self,
        note_id: NoteId,
        attachment_id: AttachmentId,
        actual: Option<Sha256Digest>,
        now: u64,
    ) -> Vec<SenderEffect> {
        let mut out = Vec::new();
        let Some(entry) = self.notes.get(&note_id) else {
            return out;
        };
        let expected = entry
            .record
            .manifest
            .attachment(attachment_id)
            .map(|a| a.sha256);
        if actual.is_some() && actual == expected {
            return out; // The file is fine: the fault was on the way. Retries continue.
        }
        self.block(note_id, BlockReason::SourceCorrupt, now, &mut out);
        out
    }

    /// Offer a blocked note again, after the owner dealt with it.
    pub fn retry_blocked(&mut self, note_id: &NoteId, now: u64) -> Vec<SenderEffect> {
        let mut out = Vec::new();
        if let Some(entry) = self.notes.get_mut(note_id) {
            if matches!(entry.record.state, OutgoingState::Blocked(_)) {
                entry.record.state = OutgoingState::Queued;
                entry.attempts = 0;
                entry.not_before = 0;
                out.push(SenderEffect::Persist(Box::new(entry.record.clone())));
                self.advance(now, &mut out);
            }
        }
        out
    }

    /// Ask the companion to speak `text`, or the note's own text if `None`. Replaces any
    /// request in flight. Returns the request id, or `None` with no link: TTS needs the
    /// companion, and the Glass has no engine to fall back on.
    pub fn request_tts(
        &mut self,
        note_id: NoteId,
        text: Option<String>,
        locale: Locale,
        now: u64,
    ) -> Option<(u32, Vec<SenderEffect>)> {
        self.link?;
        let mut out = Vec::new();
        if let Some(old) = self.tts.take() {
            out.push(SenderEffect::Send(Frame::NoteTtsRelease(NoteTtsRelease {
                request_id: old.request_id,
            })));
        }
        let request_id = self.next_tts_id;
        self.next_tts_id = self.next_tts_id.wrapping_add(1);
        self.tts = Some(TtsPull {
            request_id,
            note_id,
            ready: None,
            retries: 0,
            // Synthesis can take a while: allow a few response timeouts before the first
            // answer, which the retry loop in `tick` then counts down.
            deadline: now + self.cfg.retry.response_timeout_ms,
        });
        out.push(SenderEffect::Send(Frame::NoteTtsRequest(NoteTtsRequest {
            request_id,
            note_id,
            text,
            locale,
        })));
        Some((request_id, out))
    }

    fn advance(&mut self, now: u64, out: &mut Vec<SenderEffect>) {
        if self.link.is_none() {
            return;
        }
        if self.active.is_none() {
            let next = self
                .notes
                .values()
                .filter(|e| e.record.state == OutgoingState::Queued && e.not_before <= now)
                .min_by_key(|e| e.record.seq);
            if let Some(entry) = next {
                out.push(SenderEffect::Send(Frame::NoteOffer(NoteOffer {
                    manifest: entry.record.manifest.clone(),
                })));
                self.active = Some(Active {
                    note_id: entry.record.manifest.id,
                    deadline: now + self.cfg.retry.response_timeout_ms,
                });
            }
        }
        let stats = self.stats();
        let status = NoteSyncStatus {
            pending_notes: stats.pending_notes,
            pending_bytes: stats.pending_bytes,
            sending: self.active.is_some(),
        };
        if self.last_status != Some(status) {
            self.last_status = Some(status);
            out.push(SenderEffect::Send(Frame::NoteSyncStatus(status)));
        }
    }

    fn refresh(&mut self, note_id: &NoteId, now: u64) {
        let timeout = self.cfg.retry.response_timeout_ms;
        if let Some(active) = &mut self.active {
            if active.note_id == *note_id {
                active.deadline = now + timeout;
            }
        }
    }

    fn is_active(&self, note_id: &NoteId) -> bool {
        self.active.as_ref().map(|a| a.note_id) == Some(*note_id)
    }

    fn on_offer_reply(&mut self, reply: NoteOfferReply, now: u64, out: &mut Vec<SenderEffect>) {
        let note_id = reply.note_id;
        match reply.status {
            NoteOfferStatus::Accepted { .. } => self.refresh(&note_id, now),
            NoteOfferStatus::AlreadyComplete { manifest_sha256 } => {
                self.try_mark_synced(&note_id, &manifest_sha256, now, out)
            }
            NoteOfferStatus::Deferred { reason } => {
                let retry = self.cfg.retry;
                let Some(entry) = self.notes.get_mut(&note_id) else {
                    return;
                };
                if !matches!(entry.record.state, OutgoingState::Queued) {
                    return;
                }
                let delay = match reason {
                    // Waits for a faster link: `link_up` clears this.
                    NoteDeferReason::NeedsFasterLink { .. } => retry.backoff_max_ms,
                    _ => retry.backoff(entry.attempts),
                };
                entry.attempts += 1;
                entry.not_before = now + delay;
                out.push(SenderEffect::Event(SenderEvent::NoteDeferred {
                    note_id,
                    reason,
                }));
                if self.is_active(&note_id) {
                    self.active = None;
                }
                self.advance(now, out);
            }
            NoteOfferStatus::Rejected { reason } => {
                self.block(note_id, BlockReason::Rejected(reason), now, out)
            }
        }
    }

    fn block(
        &mut self,
        note_id: NoteId,
        reason: BlockReason,
        now: u64,
        out: &mut Vec<SenderEffect>,
    ) {
        let Some(entry) = self.notes.get_mut(&note_id) else {
            return;
        };
        if !matches!(entry.record.state, OutgoingState::Queued) {
            return;
        }
        entry.record.state = OutgoingState::Blocked(reason);
        out.push(SenderEffect::Persist(Box::new(entry.record.clone())));
        out.push(SenderEffect::Event(SenderEvent::NoteBlocked {
            note_id,
            reason,
        }));
        if self.is_active(&note_id) {
            self.active = None;
        }
        self.advance(now, out);
    }

    fn on_chunk_request(
        &mut self,
        request: NoteChunkRequest,
        now: u64,
        out: &mut Vec<SenderEffect>,
    ) {
        let NoteChunkRequest {
            note_id,
            attachment_id,
            offset,
            max_len,
        } = request;
        let unavailable = |reason| {
            SenderEffect::Send(Frame::NoteChunkUnavailable(NoteChunkUnavailable {
                note_id,
                attachment_id,
                offset,
                reason,
            }))
        };
        let Some(entry) = self.notes.get(&note_id) else {
            out.push(unavailable(NoteUnavailableReason::Unknown));
            return;
        };
        if matches!(
            entry.record.state,
            OutgoingState::Blocked(BlockReason::SourceCorrupt)
        ) {
            out.push(unavailable(NoteUnavailableReason::SourceError));
            return;
        }
        let Some(meta) = entry.record.manifest.attachment(attachment_id) else {
            out.push(unavailable(NoteUnavailableReason::Unknown));
            return;
        };
        if offset >= meta.size || max_len == 0 {
            out.push(unavailable(NoteUnavailableReason::OutOfRange));
            return;
        }
        let len = u64::from(max_len.min(MAX_NOTE_CHUNK_LEN)).min(meta.size - offset) as u32;
        self.refresh(&note_id, now);
        out.push(SenderEffect::ReadChunk {
            note_id,
            attachment_id,
            offset,
            len,
        });
    }

    fn on_attachment_ack(&mut self, ack: NoteAttachmentAck, now: u64, out: &mut Vec<SenderEffect>) {
        let NoteAttachmentAck {
            note_id,
            attachment_id,
            sha256,
            verified,
        } = ack;
        self.refresh(&note_id, now);
        let Some(entry) = self.notes.get_mut(&note_id) else {
            return;
        };
        let Some(meta) = entry.record.manifest.attachment(attachment_id) else {
            out.push(SenderEffect::Event(SenderEvent::AckIgnored { note_id }));
            return;
        };
        if !verified {
            // The companion's copy failed its whole-file check. Ours may be what is bad.
            out.push(SenderEffect::VerifySource {
                note_id,
                attachment_id,
            });
            return;
        }
        if sha256 != meta.sha256 {
            out.push(SenderEffect::Event(SenderEvent::AckIgnored { note_id }));
            return;
        }
        if entry.record.verified.insert(attachment_id) {
            out.push(SenderEffect::Persist(Box::new(entry.record.clone())));
        }
        self.request_transcripts(&note_id, out);
        self.advance(now, out);
    }

    /// The one path to `Synced`, and through it to deleting anything.
    fn try_mark_synced(
        &mut self,
        note_id: &NoteId,
        manifest_sha256: &Sha256Digest,
        now: u64,
        out: &mut Vec<SenderEffect>,
    ) {
        let delete_policy = self.cfg.delete_policy;
        let Some(entry) = self.notes.get_mut(note_id) else {
            return;
        };
        let record = &mut entry.record;
        if matches!(record.state, OutgoingState::Synced { .. }) {
            return; // A repeat.
        }
        let all_verified = record
            .manifest
            .attachments
            .iter()
            .all(|a| record.verified.contains(&a.id));
        if record.manifest_sha256 != *manifest_sha256 || !all_verified {
            out.push(SenderEffect::Event(SenderEvent::AckIgnored {
                note_id: *note_id,
            }));
            return;
        }
        let delete = delete_policy == DeletePolicy::DeleteAfterVerified;
        record.state = OutgoingState::Synced {
            delete_pending: delete,
        };
        // The decision is durable before anything is removed.
        out.push(SenderEffect::Persist(Box::new(record.clone())));
        out.push(SenderEffect::Event(SenderEvent::NoteSynced {
            note_id: *note_id,
        }));
        if delete {
            out.push(SenderEffect::DeleteLocal { note_id: *note_id });
            self.notes.remove(note_id);
        }
        if self.is_active(note_id) {
            self.active = None;
        }
        self.advance(now, out);
    }

    fn request_transcripts(&mut self, note_id: &NoteId, out: &mut Vec<SenderEffect>) {
        if self.cfg.no_auto_transcribe || self.link.is_none() {
            return;
        }
        let Some(entry) = self.notes.get(note_id) else {
            return;
        };
        let record = &entry.record;
        for meta in &record.manifest.attachments {
            let wanted = meta.kind == AttachmentKind::VoiceRecording
                && record.verified.contains(&meta.id)
                // Retried once per link after an engine failure.
                && record.transcripts.get(&meta.id).is_none_or(|t| t.error.is_some());
            if wanted && self.transcript_requested.insert((*note_id, meta.id)) {
                out.push(SenderEffect::Send(Frame::NoteTranscriptRequest(
                    NoteTranscriptRequest {
                        note_id: *note_id,
                        attachment_id: meta.id,
                    },
                )));
            }
        }
    }

    fn on_transcript(&mut self, t: NoteTranscript, out: &mut Vec<SenderEffect>) {
        let NoteTranscript {
            note_id,
            attachment_id,
            text,
            truncated,
            error,
        } = t;
        let ack = SenderEffect::Send(Frame::NoteTranscriptAck(NoteTranscriptAck {
            note_id,
            attachment_id,
        }));
        // "Not ready yet" is not a result: acknowledge, and ask again on the next ack or link.
        let not_ready = matches!(error, Some(NoteTranscriptError::AudioNotAvailable));
        if not_ready {
            self.transcript_requested.remove(&(note_id, attachment_id));
        }
        // A note already deleted here still acknowledges: the companion keeps the text.
        if let (Some(entry), false) = (self.notes.get_mut(&note_id), not_ready) {
            let transcript = ReceivedTranscript {
                text,
                truncated,
                error,
            };
            if entry.record.transcripts.get(&attachment_id) != Some(&transcript) {
                entry
                    .record
                    .transcripts
                    .insert(attachment_id, transcript.clone());
                out.push(SenderEffect::Persist(Box::new(entry.record.clone())));
                out.push(SenderEffect::Event(SenderEvent::Transcript {
                    note_id,
                    attachment_id,
                    transcript,
                }));
            }
        }
        out.push(ack);
    }

    fn tts_chunk_request(&self, request_id: u32) -> Option<Frame> {
        let tts = self.tts.as_ref().filter(|t| t.request_id == request_id)?;
        let stream = tts.ready.as_ref()?;
        let chunk_len = self.link?.chunk_len;
        Some(Frame::NoteTtsChunkRequest(NoteTtsChunkRequest {
            request_id,
            offset: stream.next_offset,
            max_len: chunk_len,
        }))
    }

    fn on_tts_ready(&mut self, ready: NoteTtsReady, now: u64, out: &mut Vec<SenderEffect>) {
        let timeout = self.cfg.retry.response_timeout_ms;
        let Some(tts) = self.tts.as_mut().filter(|t| {
            t.request_id == ready.request_id && t.note_id == ready.note_id && t.ready.is_none()
        }) else {
            return;
        };
        if ready.total_len == 0 || ready.total_len > MAX_TTS_AUDIO_BYTES {
            self.finish_tts(Some("audio size out of range".into()), out);
            return;
        }
        tts.ready = Some(TtsStream {
            sample_rate: ready.sample_rate,
            total: ready.total_len,
            sha256: ready.sha256,
            next_offset: 0,
            hasher: Sha256::new(),
        });
        tts.retries = 0;
        tts.deadline = now + timeout;
        out.extend(
            self.tts_chunk_request(ready.request_id)
                .map(SenderEffect::Send),
        );
    }

    fn on_tts_chunk(&mut self, chunk: NoteTtsChunk, now: u64, out: &mut Vec<SenderEffect>) {
        let timeout = self.cfg.retry.response_timeout_ms;
        let Some(chunk_len) = self.link.map(|l| u64::from(l.chunk_len)) else {
            return;
        };
        let Some(tts) = self
            .tts
            .as_mut()
            .filter(|t| t.request_id == chunk.request_id)
        else {
            return;
        };
        let (request_id, note_id) = (tts.request_id, tts.note_id);
        let Some(stream) = tts.ready.as_mut() else {
            return;
        };
        if chunk.offset != stream.next_offset {
            return; // A stale repeat of a chunk already taken.
        }
        let expected = chunk_len.min(stream.total - stream.next_offset);
        if chunk.data.len() as u64 != expected || sha256(&chunk.data) != chunk.sha256 {
            tts.retries += 1;
            tts.deadline = now + timeout;
            if tts.retries > MAX_TTS_RETRIES {
                self.finish_tts(Some("too many bad audio chunks".into()), out);
            } else {
                out.extend(self.tts_chunk_request(request_id).map(SenderEffect::Send));
            }
            return;
        }
        stream.hasher.update(&chunk.data);
        stream.next_offset += expected;
        tts.retries = 0;
        tts.deadline = now + timeout;
        let sample_rate = stream.sample_rate;
        let done = stream.next_offset == stream.total;
        out.push(SenderEffect::Event(SenderEvent::TtsAudio {
            request_id,
            note_id,
            sample_rate,
            data: chunk.data,
        }));
        if done {
            let matches = stream.hasher.clone().finalize().as_slice() == stream.sha256;
            let error = (!matches).then(|| "audio checksum mismatch".to_string());
            self.finish_tts(error, out);
        } else {
            out.extend(self.tts_chunk_request(request_id).map(SenderEffect::Send));
        }
    }

    fn finish_tts(&mut self, error: Option<String>, out: &mut Vec<SenderEffect>) {
        if let Some(tts) = self.tts.take() {
            out.push(SenderEffect::Send(Frame::NoteTtsRelease(NoteTtsRelease {
                request_id: tts.request_id,
            })));
            out.push(SenderEffect::Event(SenderEvent::TtsFinished {
                request_id: tts.request_id,
                note_id: tts.note_id,
                error,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omg_protocol::{AttachmentMeta, NoteAck, NoteManifest, NoteRejectReason};

    fn record(id: u8, seq: u64, sizes: &[(AttachmentKind, u64)]) -> OutgoingNote {
        let attachments: Vec<AttachmentMeta> = sizes
            .iter()
            .enumerate()
            .map(|(i, (kind, size))| AttachmentMeta {
                id: i as u16,
                kind: *kind,
                mime: "x/y".into(),
                size: *size,
                sha256: [i as u8 + 1; 32],
                duration_ms: None,
            })
            .collect();
        let manifest = NoteManifest {
            id: NoteId([id; 16]),
            created_at_ms: 0,
            locale: Locale::It,
            text: Some("t".into()),
            attachments,
        };
        OutgoingNote {
            format: 1,
            seq,
            manifest_sha256: crate::notes::manifest_digest(&manifest),
            manifest,
            verified: BTreeSet::new(),
            state: OutgoingState::Queued,
            transcripts: BTreeMap::new(),
        }
    }

    fn sender(policy: DeletePolicy, records: Vec<OutgoingNote>) -> NoteSender {
        NoteSender::new(
            SenderConfig {
                delete_policy: policy,
                ..Default::default()
            },
            records,
        )
    }

    fn offered(effects: &[SenderEffect]) -> Vec<NoteId> {
        effects
            .iter()
            .filter_map(|e| match e {
                SenderEffect::Send(Frame::NoteOffer(o)) => Some(o.manifest.id),
                _ => None,
            })
            .collect()
    }

    fn deletes(effects: &[SenderEffect]) -> usize {
        effects
            .iter()
            .filter(|e| matches!(e, SenderEffect::DeleteLocal { .. }))
            .count()
    }

    #[test]
    fn a_note_can_be_forgotten_unless_it_is_being_sent() {
        let sizes = [(AttachmentKind::Photo, 10)];
        let mut s = sender(
            DeletePolicy::default(),
            vec![record(1, 0, &sizes), record(2, 1, &sizes)],
        );
        assert!(s.forget(&NoteId([9; 16])).is_none(), "unknown");
        s.link_up(LinkProfile::fast(), 0);
        assert!(s.forget(&NoteId([1; 16])).is_none(), "on offer");
        assert_eq!(deletes(&s.forget(&NoteId([2; 16])).unwrap()), 1);
        assert!(s.note(&NoteId([2; 16])).is_none());
        assert!(s.note(&NoteId([1; 16])).is_some());
    }

    fn ack(note: u8, att: u16, sha: u8) -> Frame {
        Frame::NoteAttachmentAck(NoteAttachmentAck {
            note_id: NoteId([note; 16]),
            attachment_id: att,
            sha256: [sha; 32],
            verified: true,
        })
    }

    #[test]
    fn nothing_is_sent_without_a_link_and_offers_follow_queue_order_not_ids() {
        let mut s = sender(
            DeletePolicy::Keep,
            vec![
                record(9, 0, &[(AttachmentKind::Photo, 10)]),
                record(1, 1, &[(AttachmentKind::Photo, 10)]),
            ],
        );
        assert!(s.tick(1_000).is_empty());
        assert_eq!(
            offered(&s.link_up(LinkProfile::fast(), 1_000)),
            vec![NoteId([9; 16])],
            "the oldest first, even with the larger id, and one at a time"
        );
    }

    #[test]
    fn enqueue_while_offline_waits_and_syncs_on_link_up() {
        let mut s = sender(DeletePolicy::Keep, vec![]);
        let r = record(1, 0, &[(AttachmentKind::Photo, 10)]);
        assert!(s.enqueue(r, 5).is_empty());
        assert_eq!(s.stats().pending_notes, 1);
        assert_eq!(s.stats().pending_bytes, 10);
        assert_eq!(offered(&s.link_up(LinkProfile::fast(), 6)).len(), 1);
    }

    #[test]
    fn delete_needs_every_attachment_ack_and_the_matching_digest() {
        let r = record(
            1,
            0,
            &[(AttachmentKind::Photo, 10), (AttachmentKind::Photo, 20)],
        );
        let digest = r.manifest_sha256;
        let id = r.manifest.id;
        let mut s = sender(DeletePolicy::DeleteAfterVerified, vec![r]);
        s.link_up(LinkProfile::fast(), 0);
        let good = Frame::NoteAck(NoteAck {
            note_id: id,
            manifest_sha256: digest,
        });

        // Too early: no attachment acknowledged.
        let e = s.on_frame(good.clone(), 1);
        assert_eq!(deletes(&e), 0);
        assert!(e.contains(&SenderEffect::Event(SenderEvent::AckIgnored {
            note_id: id
        })));

        // An attachment ack whose digest does not match the manifest counts for nothing.
        let e = s.on_frame(ack(1, 0, 0xee), 2);
        assert!(e.contains(&SenderEffect::Event(SenderEvent::AckIgnored {
            note_id: id
        })));
        assert!(s.note(&id).unwrap().verified.is_empty());

        // verified: false counts for nothing, and asks for a look at the local file.
        let e = s.on_frame(
            Frame::NoteAttachmentAck(NoteAttachmentAck {
                note_id: id,
                attachment_id: 0,
                sha256: [1; 32],
                verified: false,
            }),
            3,
        );
        assert!(matches!(e[0], SenderEffect::VerifySource { .. }));
        assert!(s.note(&id).unwrap().verified.is_empty());

        // One of two is still too early.
        s.on_frame(ack(1, 0, 1), 4);
        assert_eq!(deletes(&s.on_frame(good.clone(), 5)), 0);

        // All acked, but the wrong manifest digest.
        s.on_frame(ack(1, 1, 2), 6);
        let e = s.on_frame(
            Frame::NoteAck(NoteAck {
                note_id: id,
                manifest_sha256: [0; 32],
            }),
            7,
        );
        assert_eq!(deletes(&e), 0);
        assert_eq!(s.note(&id).unwrap().state, OutgoingState::Queued);

        // Now it holds, and the decision is persisted before the delete.
        let e = s.on_frame(good, 8);
        let persist = e
            .iter()
            .position(|e| matches!(e, SenderEffect::Persist(_)))
            .unwrap();
        let delete = e
            .iter()
            .position(|e| matches!(e, SenderEffect::DeleteLocal { .. }))
            .unwrap();
        assert!(persist < delete);
    }

    #[test]
    fn keep_policy_never_deletes_and_a_repeated_ack_is_harmless() {
        let r = record(1, 0, &[(AttachmentKind::Photo, 10)]);
        let digest = r.manifest_sha256;
        let id = r.manifest.id;
        let mut s = sender(DeletePolicy::Keep, vec![r]);
        s.link_up(LinkProfile::fast(), 0);
        s.on_frame(ack(1, 0, 1), 1);
        let good = Frame::NoteAck(NoteAck {
            note_id: id,
            manifest_sha256: digest,
        });
        let first = s.on_frame(good.clone(), 2);
        assert_eq!(deletes(&first), 0);
        assert_eq!(
            s.note(&id).unwrap().state,
            OutgoingState::Synced {
                delete_pending: false
            }
        );
        assert!(s.on_frame(good, 3).is_empty());
        assert_eq!(s.stats(), QueueStats::default());
    }

    #[test]
    fn a_delete_interrupted_by_a_crash_finishes_at_startup_and_only_that() {
        let mut due = record(1, 0, &[(AttachmentKind::Photo, 10)]);
        due.state = OutgoingState::Synced {
            delete_pending: true,
        };
        let mut kept = record(2, 1, &[(AttachmentKind::Photo, 10)]);
        kept.state = OutgoingState::Synced {
            delete_pending: false,
        };
        let queued = record(3, 2, &[(AttachmentKind::Photo, 10)]);
        let mut s = sender(DeletePolicy::DeleteAfterVerified, vec![due, kept, queued]);
        assert_eq!(
            s.startup(),
            vec![SenderEffect::DeleteLocal {
                note_id: NoteId([1; 16])
            }]
        );
    }

    #[test]
    fn silence_repeats_the_offer_with_growing_gaps() {
        let mut s = sender(
            DeletePolicy::Keep,
            vec![record(1, 0, &[(AttachmentKind::Photo, 10)])],
        );
        s.link_up(LinkProfile::fast(), 0);
        let t1 = s.next_deadline().unwrap();
        assert_eq!(t1, 30_000);
        assert!(offered(&s.tick(t1 - 1)).is_empty());
        assert_eq!(offered(&s.tick(t1)).len(), 1);
        let t2 = s.next_deadline().unwrap();
        assert!(t2 - t1 > 30_000);
        // A chunk request shows the peer is alive and pushes the deadline back.
        s.on_frame(
            Frame::NoteChunkRequest(NoteChunkRequest {
                note_id: NoteId([1; 16]),
                attachment_id: 0,
                offset: 0,
                max_len: 4,
            }),
            t2 - 1,
        );
        assert!(offered(&s.tick(t2)).is_empty());
    }

    #[test]
    fn deferred_parks_and_moves_on_then_a_new_link_clears_the_delay() {
        let mut s = sender(
            DeletePolicy::Keep,
            vec![
                record(1, 0, &[(AttachmentKind::Video, 10)]),
                record(2, 1, &[(AttachmentKind::Photo, 10)]),
            ],
        );
        s.link_up(LinkProfile::moderate(), 0);
        let e = s.on_frame(
            Frame::NoteOfferReply(NoteOfferReply {
                note_id: NoteId([1; 16]),
                status: NoteOfferStatus::Deferred {
                    reason: NoteDeferReason::NeedsFasterLink { bytes: 10 },
                },
            }),
            1,
        );
        assert_eq!(offered(&e), vec![NoteId([2; 16])]);
        assert_eq!(
            s.note(&NoteId([1; 16])).unwrap().state,
            OutgoingState::Queued
        );
        s.link_down();
        assert_eq!(
            offered(&s.link_up(LinkProfile::fast(), 2)),
            vec![NoteId([1; 16])]
        );
    }

    #[test]
    fn rejected_keeps_the_note_blocked_until_retried() {
        let mut s = sender(
            DeletePolicy::DeleteAfterVerified,
            vec![record(1, 0, &[(AttachmentKind::Photo, 10)])],
        );
        s.link_up(LinkProfile::fast(), 0);
        let id = NoteId([1; 16]);
        let e = s.on_frame(
            Frame::NoteOfferReply(NoteOfferReply {
                note_id: id,
                status: NoteOfferStatus::Rejected {
                    reason: NoteRejectReason::ManifestConflict,
                },
            }),
            1,
        );
        assert_eq!(deletes(&e), 0);
        assert!(matches!(
            s.note(&id).unwrap().state,
            OutgoingState::Blocked(_)
        ));
        assert!(offered(&s.tick(10_000_000)).is_empty());
        assert_eq!(offered(&s.retry_blocked(&id, 10_000_001)), vec![id]);
    }

    #[test]
    fn chunk_requests_are_clamped_and_checked() {
        let r = record(1, 0, &[(AttachmentKind::Video, 100_000)]);
        let id = r.manifest.id;
        let mut s = sender(DeletePolicy::Keep, vec![r]);
        s.link_up(LinkProfile::fast(), 0);
        let ask = |offset, max_len, attachment_id, note_id| {
            Frame::NoteChunkRequest(NoteChunkRequest {
                note_id,
                attachment_id,
                offset,
                max_len,
            })
        };
        // Oversized request: clamped to the protocol cap.
        let e = s.on_frame(ask(0, u32::MAX, 0, id), 1);
        assert!(matches!(
            e[0],
            SenderEffect::ReadChunk { len, .. } if len == MAX_NOTE_CHUNK_LEN
        ));
        // Near the end: only what exists.
        let e = s.on_frame(ask(99_999, 4096, 0, id), 2);
        assert!(matches!(e[0], SenderEffect::ReadChunk { len: 1, .. }));
        for (frame, reason) in [
            (ask(100_000, 1, 0, id), NoteUnavailableReason::OutOfRange),
            (ask(u64::MAX, 1, 0, id), NoteUnavailableReason::OutOfRange),
            (ask(0, 0, 0, id), NoteUnavailableReason::OutOfRange),
            (ask(0, 1, 7, id), NoteUnavailableReason::Unknown),
            (
                ask(0, 1, 0, NoteId([5; 16])),
                NoteUnavailableReason::Unknown,
            ),
        ] {
            let e = s.on_frame(frame, 3);
            assert!(
                matches!(&e[0], SenderEffect::Send(Frame::NoteChunkUnavailable(u)) if u.reason == reason),
                "{e:?}"
            );
        }
    }

    #[test]
    fn a_failed_read_checks_the_source_and_a_bad_file_blocks_but_is_kept() {
        let r = record(1, 0, &[(AttachmentKind::Photo, 10)]);
        let id = r.manifest.id;
        let mut s = sender(DeletePolicy::DeleteAfterVerified, vec![r]);
        s.link_up(LinkProfile::fast(), 0);
        let e = s.on_chunk_read(id, 0, 0, Err(()));
        assert!(matches!(e[1], SenderEffect::VerifySource { .. }));
        // The file matches: nothing changes.
        assert!(s.on_source_checked(id, 0, Some([1; 32]), 1).is_empty());
        // It does not: blocked, kept, and it serves no more chunks.
        let e = s.on_source_checked(id, 0, Some([9; 32]), 2);
        assert_eq!(deletes(&e), 0);
        assert_eq!(
            s.note(&id).unwrap().state,
            OutgoingState::Blocked(BlockReason::SourceCorrupt)
        );
    }

    #[test]
    fn transcript_is_requested_once_per_link_after_the_voice_ack() {
        let r = record(
            1,
            0,
            &[
                (AttachmentKind::VoiceRecording, 10),
                (AttachmentKind::Photo, 5),
            ],
        );
        let id = r.manifest.id;
        let mut s = sender(DeletePolicy::Keep, vec![r]);
        s.link_up(LinkProfile::fast(), 0);
        let requests = |e: &[SenderEffect]| {
            e.iter()
                .filter(|e| matches!(e, SenderEffect::Send(Frame::NoteTranscriptRequest(_))))
                .count()
        };
        assert_eq!(requests(&s.on_frame(ack(1, 1, 2), 1)), 0, "a photo");
        assert_eq!(requests(&s.on_frame(ack(1, 0, 1), 2)), 1, "the recording");
        assert_eq!(requests(&s.on_frame(ack(1, 0, 1), 3)), 0, "a repeat");

        // Failure comes back: acknowledged, and asked again on the next link only.
        let failure = NoteTranscript {
            note_id: id,
            attachment_id: 0,
            text: String::new(),
            truncated: false,
            error: Some(NoteTranscriptError::EngineUnavailable),
        };
        let e = s.on_frame(Frame::NoteTranscript(failure), 4);
        assert!(matches!(
            e.last(),
            Some(SenderEffect::Send(Frame::NoteTranscriptAck(_)))
        ));
        assert_eq!(requests(&e), 0);
        s.link_down();
        assert_eq!(requests(&s.link_up(LinkProfile::fast(), 5)), 1);

        // A real one ends it, and is acknowledged even after a repeat.
        let good = NoteTranscript {
            note_id: id,
            attachment_id: 0,
            text: "ciao".into(),
            truncated: false,
            error: None,
        };
        let e = s.on_frame(Frame::NoteTranscript(good.clone()), 6);
        assert!(e.iter().any(|e| matches!(e, SenderEffect::Persist(_))));
        let e = s.on_frame(Frame::NoteTranscript(good), 7);
        assert_eq!(e.len(), 1, "a repeat only re-acknowledges");
        s.link_down();
        assert_eq!(requests(&s.link_up(LinkProfile::fast(), 8)), 0);
    }

    #[test]
    fn a_transcript_for_a_deleted_note_is_still_acknowledged() {
        let mut s = sender(DeletePolicy::Keep, vec![]);
        let e = s.on_frame(
            Frame::NoteTranscript(NoteTranscript {
                note_id: NoteId([1; 16]),
                attachment_id: 0,
                text: "x".into(),
                truncated: false,
                error: None,
            }),
            0,
        );
        assert!(matches!(
            e[..],
            [SenderEffect::Send(Frame::NoteTranscriptAck(_))]
        ));
    }

    #[test]
    fn tts_needs_a_link_and_verifies_what_it_plays() {
        let mut s = sender(DeletePolicy::Keep, vec![]);
        let id = NoteId([1; 16]);
        assert!(s.request_tts(id, None, Locale::It, 0).is_none());

        s.link_up(
            LinkProfile {
                chunk_len: 2,
                window: 1,
                max_auto_attachment_bytes: None,
            },
            0,
        );
        let (rid, _) = s.request_tts(id, Some("x".into()), Locale::It, 0).unwrap();
        let pcm = [1u8, 2, 3];
        let e = s.on_frame(
            Frame::NoteTtsReady(NoteTtsReady {
                request_id: rid,
                note_id: id,
                sample_rate: 16_000,
                total_len: 3,
                sha256: sha256(&pcm),
            }),
            1,
        );
        assert!(matches!(
            e[0],
            SenderEffect::Send(Frame::NoteTtsChunkRequest(NoteTtsChunkRequest {
                offset: 0,
                max_len: 2,
                ..
            }))
        ));
        // A corrupt chunk is not played; it is asked for again.
        let e = s.on_frame(
            Frame::NoteTtsChunk(NoteTtsChunk {
                request_id: rid,
                offset: 0,
                data: vec![9, 9],
                sha256: sha256(&pcm[..2]),
            }),
            2,
        );
        assert_eq!(e.len(), 1);
        assert!(matches!(
            e[0],
            SenderEffect::Send(Frame::NoteTtsChunkRequest(_))
        ));

        let chunk = |offset: u64, data: &[u8]| {
            Frame::NoteTtsChunk(NoteTtsChunk {
                request_id: rid,
                offset,
                data: data.to_vec(),
                sha256: sha256(data),
            })
        };
        let e = s.on_frame(chunk(0, &pcm[..2]), 3);
        assert!(matches!(
            &e[0],
            SenderEffect::Event(SenderEvent::TtsAudio { data, .. }) if data == &pcm[..2]
        ));
        let e = s.on_frame(chunk(2, &pcm[2..]), 4);
        assert!(e.contains(&SenderEffect::Event(SenderEvent::TtsFinished {
            request_id: rid,
            note_id: id,
            error: None,
        })));
        assert!(
            e.contains(&SenderEffect::Send(Frame::NoteTtsRelease(NoteTtsRelease {
                request_id: rid,
            })))
        );
    }
}
