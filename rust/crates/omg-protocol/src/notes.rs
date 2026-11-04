//! Notes with attachments and deferred sync: the wire schema.
//!
//! A note is taken on the Glass and stays there until the companion holds a verified
//! copy. The companion **pulls**: the Glass offers one note, the companion asks for
//! bytes at its own pace, checks them, and acknowledges only what is durable. See
//! `docs/companion-notes-sync.md`.
//!
//! Compatibility rules:
//!
//! * postcard is positional, so structs here cannot gain fields and enums can only gain
//!   variants at the end. A change beyond that is a new [`crate::Frame`] variant.
//! * A peer built before these frames existed treats them as a fatal decode error and
//!   closes the link. Never send one unless the peer advertised
//!   [`crate::capability::NOTES_SYNC`] in its `Hello`.
//! * Nothing here names a transport. Chunk size is chosen per request by the side that
//!   pulls, from what it knows about its link.

use serde::{Deserialize, Serialize};

use crate::Locale;

/// Largest chunk a peer may ask for or send. With its headers a chunk stays under the
/// 64 KiB frame size the Bluetooth path is designed around (see `docs/media-sync.md`).
pub const MAX_NOTE_CHUNK_LEN: u32 = 48 * 1024;
/// Largest note body text, bytes of UTF-8.
pub const MAX_NOTE_TEXT_BYTES: usize = 32 * 1024;
/// Largest transcript in a frame, bytes of UTF-8. The companion keeps the full text.
pub const MAX_TRANSCRIPT_BYTES: usize = 48 * 1024;
/// Largest voice recording: about an hour of 16 kHz 16-bit mono PCM.
pub const MAX_VOICE_BYTES: u64 = 128 * 1024 * 1024;
/// Largest single photo.
pub const MAX_PHOTO_BYTES: u64 = 32 * 1024 * 1024;
/// Largest video.
pub const MAX_VIDEO_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Most photos on one note.
pub const MAX_PHOTOS_PER_NOTE: usize = 32;
/// Whether one note may carry photos and a video together. Owner default, still open:
/// see `docs/companion-notes-sync.md`. Set to `false` to allow one or the other only.
pub const ALLOW_PHOTOS_WITH_VIDEO: bool = true;
/// Largest sum of attachment sizes on one note.
pub const MAX_NOTE_TOTAL_BYTES: u64 = 3 * 1024 * 1024 * 1024;
/// Longest attachment MIME type, bytes.
pub const MAX_MIME_LEN: usize = 64;
/// Longest text one TTS request may carry, bytes of UTF-8.
pub const MAX_TTS_TEXT_BYTES: usize = 16 * 1024;
/// Largest synthesized audio served for one TTS request.
pub const MAX_TTS_AUDIO_BYTES: u64 = 32 * 1024 * 1024;

/// A SHA-256 digest.
pub type Sha256Digest = [u8; 32];

/// Identifies an attachment within its note.
pub type AttachmentId = u16;

/// A note's id: 128 random bits chosen by the Glass when the note is sealed.
///
/// A fixed-size value on purpose: storage paths are derived from its hex form, so a
/// peer can never smuggle a path through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NoteId(pub [u8; 16]);

impl NoteId {
    /// Lowercase hex, 32 characters. Safe to use as a file name.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Parses [`to_hex`](NoteId::to_hex) output. Accepts lowercase only, so one id has
    /// exactly one spelling on disk.
    pub fn from_hex(s: &str) -> Option<NoteId> {
        let bytes = s.as_bytes();
        if bytes.len() != 32 {
            return None;
        }
        let nibble = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        let mut out = [0u8; 16];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = (nibble(bytes[i * 2])? << 4) | nibble(bytes[i * 2 + 1])?;
        }
        Some(NoteId(out))
    }
}

/// What an attachment is. Limits and sync order depend on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AttachmentKind {
    /// The audio of a voice note. Transcribed on the companion, never on the Glass.
    VoiceRecording,
    /// A photo.
    Photo,
    /// A video.
    Video,
}

impl AttachmentKind {
    /// Largest size allowed for this kind.
    pub fn max_bytes(self) -> u64 {
        match self {
            AttachmentKind::VoiceRecording => MAX_VOICE_BYTES,
            AttachmentKind::Photo => MAX_PHOTO_BYTES,
            AttachmentKind::Video => MAX_VIDEO_BYTES,
        }
    }
}

/// One attachment as described in a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentMeta {
    /// Unique within the note.
    pub id: AttachmentId,
    /// What it is.
    pub kind: AttachmentKind,
    /// MIME type, e.g. `audio/wav`.
    pub mime: String,
    /// Exact size in bytes (at least 1).
    pub size: u64,
    /// SHA-256 of the whole file.
    pub sha256: Sha256Digest,
    /// Length of audio or video, when known.
    pub duration_ms: Option<u32>,
}

/// Everything about a note except its bytes. Immutable once the note is sealed: the
/// same id with a different manifest is a conflict, never an update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteManifest {
    /// The note's id.
    pub id: NoteId,
    /// Glass wall clock when taken, Unix ms. Informational: the Glass clock may be
    /// wrong before its first time sync, so nothing orders by this.
    pub created_at_ms: u64,
    /// Language of the note; the companion transcribes in it.
    pub locale: Locale,
    /// Typed or pre-existing text, if any.
    pub text: Option<String>,
    /// Attachments, possibly none.
    pub attachments: Vec<AttachmentMeta>,
}

/// Why a manifest is not acceptable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// No text and no attachments.
    Empty,
    /// The text is over [`MAX_NOTE_TEXT_BYTES`].
    TextTooLong,
    /// Two attachments share an id.
    DuplicateAttachmentId(AttachmentId),
    /// More than one voice recording.
    TooManyVoiceRecordings,
    /// More than [`MAX_PHOTOS_PER_NOTE`] photos.
    TooManyPhotos,
    /// More than one video.
    TooManyVideos,
    /// Photos and a video on one note, while [`ALLOW_PHOTOS_WITH_VIDEO`] is off.
    PhotosAndVideo,
    /// An attachment is empty or over its kind's limit.
    BadAttachmentSize(AttachmentId),
    /// A MIME type is empty, too long or not printable ASCII.
    BadMime(AttachmentId),
    /// The attachments add up to more than [`MAX_NOTE_TOTAL_BYTES`].
    TotalTooLarge,
}

impl NoteManifest {
    /// Checks the protocol limits. Both sides run this: the Glass before sealing, the
    /// companion on every offer.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.attachments.is_empty() && self.text.as_deref().unwrap_or("").is_empty() {
            return Err(ManifestError::Empty);
        }
        if self.text.as_deref().map_or(0, str::len) > MAX_NOTE_TEXT_BYTES {
            return Err(ManifestError::TextTooLong);
        }
        let (mut voice, mut photos, mut videos, mut total) = (0usize, 0usize, 0usize, 0u64);
        for (i, a) in self.attachments.iter().enumerate() {
            if self.attachments[..i].iter().any(|b| b.id == a.id) {
                return Err(ManifestError::DuplicateAttachmentId(a.id));
            }
            match a.kind {
                AttachmentKind::VoiceRecording => voice += 1,
                AttachmentKind::Photo => photos += 1,
                AttachmentKind::Video => videos += 1,
            }
            if a.size == 0 || a.size > a.kind.max_bytes() {
                return Err(ManifestError::BadAttachmentSize(a.id));
            }
            let mime_ok = !a.mime.is_empty()
                && a.mime.len() <= MAX_MIME_LEN
                && a.mime.bytes().all(|c| c.is_ascii_graphic());
            if !mime_ok {
                return Err(ManifestError::BadMime(a.id));
            }
            total = total
                .checked_add(a.size)
                .ok_or(ManifestError::TotalTooLarge)?;
        }
        if voice > 1 {
            return Err(ManifestError::TooManyVoiceRecordings);
        }
        if photos > MAX_PHOTOS_PER_NOTE {
            return Err(ManifestError::TooManyPhotos);
        }
        if videos > 1 {
            return Err(ManifestError::TooManyVideos);
        }
        if !ALLOW_PHOTOS_WITH_VIDEO && photos > 0 && videos > 0 {
            return Err(ManifestError::PhotosAndVideo);
        }
        if total > MAX_NOTE_TOTAL_BYTES {
            return Err(ManifestError::TotalTooLarge);
        }
        Ok(())
    }

    /// The bytes the manifest digest is computed over: the `manifest_sha256` in
    /// acknowledgements is the SHA-256 of these.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        // Plain data with no maps or floats: encoding cannot fail and is deterministic.
        postcard::to_allocvec(self).expect("a manifest always serializes")
    }

    /// The attachment with this id.
    pub fn attachment(&self, id: AttachmentId) -> Option<&AttachmentMeta> {
        self.attachments.iter().find(|a| a.id == id)
    }
}

/// Glass -> Companion: this note is waiting. Sent when a link comes up and again on
/// retry; repeating it never resets progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteOffer {
    /// The note.
    pub manifest: NoteManifest,
}

/// How far the companion has one attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentProgress {
    /// The attachment.
    pub attachment_id: AttachmentId,
    /// Bytes already held; the transfer resumes here.
    pub next_offset: u64,
    /// The whole file is durable and its SHA-256 matched.
    pub verified: bool,
}

/// Why the companion cannot take a note right now. The note stays queued on the Glass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteDeferReason {
    /// The companion lacks the space, or a write failed.
    InsufficientStorage,
    /// What is left is too big for this link. Sync resumes on a faster one.
    NeedsFasterLink {
        /// Size of the smallest attachment that was held back.
        bytes: u64,
    },
    /// Repeated checksum failures; the sender should check its copy.
    IntegrityFailure,
}

/// Why the companion refuses a note outright. The Glass keeps it and tells the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteRejectReason {
    /// The manifest breaks a protocol limit.
    InvalidManifest,
    /// The companion holds this id with a different manifest.
    ManifestConflict,
}

/// The companion's answer to a [`NoteOffer`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteOfferStatus {
    /// Transfer in progress or about to start, from these offsets.
    Accepted {
        /// One entry per attachment.
        progress: Vec<AttachmentProgress>,
    },
    /// Already held and verified. Preceded by a [`NoteAttachmentAck`] per attachment.
    AlreadyComplete {
        /// Digest of the manifest the companion holds.
        manifest_sha256: Sha256Digest,
    },
    /// Not now.
    Deferred {
        /// Why.
        reason: NoteDeferReason,
    },
    /// Not ever, as offered.
    Rejected {
        /// Why.
        reason: NoteRejectReason,
    },
}

/// Companion -> Glass: answer to an offer. Also sent unprompted as `Deferred` when a
/// transfer has to pause, so the Glass can move on to its next note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteOfferReply {
    /// The note.
    pub note_id: NoteId,
    /// The answer.
    pub status: NoteOfferStatus,
}

/// Companion -> Glass: send these bytes. The pulling side picks `max_len` to suit its
/// link and may have a few of these outstanding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteChunkRequest {
    /// The note.
    pub note_id: NoteId,
    /// The attachment.
    pub attachment_id: AttachmentId,
    /// Where to read from.
    pub offset: u64,
    /// Bytes wanted, at most [`MAX_NOTE_CHUNK_LEN`]. The answer holds exactly
    /// `min(max_len, size - offset)` bytes.
    pub max_len: u32,
}

/// Glass -> Companion: bytes of an attachment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteChunk {
    /// The note.
    pub note_id: NoteId,
    /// The attachment.
    pub attachment_id: AttachmentId,
    /// Where these bytes go.
    pub offset: u64,
    /// The bytes.
    pub data: Vec<u8>,
    /// SHA-256 of `data`.
    pub sha256: Sha256Digest,
}

/// Why a chunk was thrown away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteChunkRejectReason {
    /// `sha256` does not match `data`.
    Corrupt,
    /// `data` is not the length the request implied.
    BadLength,
    /// A chunk was skipped: bytes before `offset` never arrived.
    Gap,
}

/// Companion -> Glass: this chunk was discarded and will be requested again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteChunkReject {
    /// The note.
    pub note_id: NoteId,
    /// The attachment.
    pub attachment_id: AttachmentId,
    /// The chunk's offset.
    pub offset: u64,
    /// Why.
    pub reason: NoteChunkRejectReason,
}

/// Why the Glass cannot serve a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteUnavailableReason {
    /// No such note or attachment here.
    Unknown,
    /// The offset or length is out of range.
    OutOfRange,
    /// The file could not be read, or no longer matches its manifest.
    SourceError,
}

/// Glass -> Companion: answer to a chunk request that cannot be served.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteChunkUnavailable {
    /// The note.
    pub note_id: NoteId,
    /// The attachment.
    pub attachment_id: AttachmentId,
    /// The requested offset.
    pub offset: u64,
    /// Why.
    pub reason: NoteUnavailableReason,
}

/// Companion -> Glass: verdict on a whole attachment.
///
/// `verified: true` is sent only after the file is durable on the companion and its
/// SHA-256 matched. The Glass also compares `sha256` with its own manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteAttachmentAck {
    /// The note.
    pub note_id: NoteId,
    /// The attachment.
    pub attachment_id: AttachmentId,
    /// SHA-256 the companion computed over what it holds.
    pub sha256: Sha256Digest,
    /// Whether that matched the manifest.
    pub verified: bool,
}

/// Companion -> Glass: the whole note is durable and verified. The Glass may act on
/// this only if the digest matches and it holds a verified ack for every attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteAck {
    /// The note.
    pub note_id: NoteId,
    /// SHA-256 of [`NoteManifest::canonical_bytes`].
    pub manifest_sha256: Sha256Digest,
}

/// Glass -> Companion: what is still waiting. For the UI, and to tell when a sync is
/// finished. Sent when the link comes up and whenever it changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteSyncStatus {
    /// Notes not yet fully verified on the companion.
    pub pending_notes: u32,
    /// Bytes of their attachments not yet verified.
    pub pending_bytes: u64,
    /// Whether a note is on offer right now.
    pub sending: bool,
}

/// Glass -> Companion: transcribe this note's recording. Idempotent. The companion
/// runs the speech-to-text engine; the Glass never does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteTranscriptRequest {
    /// The note.
    pub note_id: NoteId,
    /// The voice recording.
    pub attachment_id: AttachmentId,
}

/// Why a note has no transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteTranscriptError {
    /// The companion does not hold the verified recording yet.
    AudioNotAvailable,
    /// The attachment is not a voice recording, or is unknown.
    NotAVoiceRecording,
    /// This companion has no speech-to-text engine (or none for this language).
    EngineUnavailable,
    /// The engine failed.
    EngineFailed(String),
}

/// Companion -> Glass: the transcript of a voice recording, or why there is none.
/// Sent until acknowledged, including across reconnects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteTranscript {
    /// The note.
    pub note_id: NoteId,
    /// The voice recording.
    pub attachment_id: AttachmentId,
    /// The text, cut at [`MAX_TRANSCRIPT_BYTES`]. Empty on error.
    pub text: String,
    /// The companion's copy is longer than `text`.
    pub truncated: bool,
    /// Set when there is no transcript.
    pub error: Option<NoteTranscriptError>,
}

/// Glass -> Companion: stop resending this transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteTranscriptAck {
    /// The note.
    pub note_id: NoteId,
    /// The voice recording.
    pub attachment_id: AttachmentId,
}

/// Glass -> Companion: read this aloud. The companion runs the TTS engine; the Glass
/// only plays the result. Not queued: if the link drops, the Glass asks again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteTtsRequest {
    /// Chosen by the Glass; a repeat of the same id is not synthesized twice.
    pub request_id: u32,
    /// The note this belongs to.
    pub note_id: NoteId,
    /// What to say, at most [`MAX_TTS_TEXT_BYTES`]. `None` means the note's own text or
    /// transcript as the companion holds it.
    pub text: Option<String>,
    /// Language to speak.
    pub locale: Locale,
}

/// Companion -> Glass: audio is ready to pull. PCM 16-bit little-endian mono.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteTtsReady {
    /// Matches the request.
    pub request_id: u32,
    /// The note.
    pub note_id: NoteId,
    /// PCM sample rate.
    pub sample_rate: u32,
    /// Total audio bytes.
    pub total_len: u64,
    /// SHA-256 of all of it.
    pub sha256: Sha256Digest,
}

/// Companion -> Glass: no audio for this request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteTtsFailed {
    /// Matches the request.
    pub request_id: u32,
    /// The note.
    pub note_id: NoteId,
    /// Why.
    pub reason: String,
}

/// Glass -> Companion: send these audio bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteTtsChunkRequest {
    /// Matches the request.
    pub request_id: u32,
    /// Where to read from.
    pub offset: u64,
    /// Bytes wanted, at most [`MAX_NOTE_CHUNK_LEN`].
    pub max_len: u32,
}

/// Companion -> Glass: audio bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteTtsChunk {
    /// Matches the request.
    pub request_id: u32,
    /// Where these bytes go.
    pub offset: u64,
    /// The bytes.
    pub data: Vec<u8>,
    /// SHA-256 of `data`.
    pub sha256: Sha256Digest,
}

/// Glass -> Companion: finished with this audio; the companion can free it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteTtsRelease {
    /// Matches the request.
    pub request_id: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{wire, Frame};

    fn meta(id: AttachmentId, kind: AttachmentKind, size: u64) -> AttachmentMeta {
        AttachmentMeta {
            id,
            kind,
            mime: "application/octet-stream".into(),
            size,
            sha256: [7; 32],
            duration_ms: None,
        }
    }

    fn manifest(attachments: Vec<AttachmentMeta>) -> NoteManifest {
        NoteManifest {
            id: NoteId([1; 16]),
            created_at_ms: 5,
            locale: Locale::It,
            text: None,
            attachments,
        }
    }

    #[test]
    fn note_id_hex_roundtrips_and_refuses_anything_path_like() {
        let id = NoteId([0xab; 16]);
        assert_eq!(NoteId::from_hex(&id.to_hex()), Some(id));
        assert_eq!(NoteId::from_hex("../../../../etc/passwd.........."), None);
        assert_eq!(NoteId::from_hex(&id.to_hex().to_uppercase()), None);
        assert_eq!(NoteId::from_hex("abc"), None);
    }

    #[test]
    fn manifest_limits() {
        use AttachmentKind::*;
        assert_eq!(manifest(vec![]).validate(), Err(ManifestError::Empty));
        let mut with_text = manifest(vec![]);
        with_text.text = Some("hello".into());
        assert_eq!(with_text.validate(), Ok(()));
        with_text.text = Some("x".repeat(MAX_NOTE_TEXT_BYTES + 1));
        assert_eq!(with_text.validate(), Err(ManifestError::TextTooLong));

        // Voice with photos, a video, or (see ALLOW_PHOTOS_WITH_VIDEO) both.
        let voice = || meta(0, VoiceRecording, 10);
        assert_eq!(
            manifest(vec![voice(), meta(1, Photo, 1), meta(2, Photo, 1)]).validate(),
            Ok(())
        );
        assert_eq!(
            manifest(vec![voice(), meta(1, Video, 1)]).validate(),
            Ok(())
        );

        let mixed = manifest(vec![meta(1, Photo, 1), meta(2, Video, 1)]).validate();
        if ALLOW_PHOTOS_WITH_VIDEO {
            assert_eq!(mixed, Ok(()));
        } else {
            assert_eq!(mixed, Err(ManifestError::PhotosAndVideo));
        }
        assert_eq!(
            manifest(vec![meta(1, Video, 1), meta(2, Video, 1)]).validate(),
            Err(ManifestError::TooManyVideos)
        );
        assert_eq!(
            manifest(vec![voice(), meta(1, VoiceRecording, 1)]).validate(),
            Err(ManifestError::TooManyVoiceRecordings)
        );
        assert_eq!(
            manifest(vec![meta(1, Photo, 1), meta(1, Photo, 1)]).validate(),
            Err(ManifestError::DuplicateAttachmentId(1))
        );
        assert_eq!(
            manifest(vec![meta(1, Photo, 0)]).validate(),
            Err(ManifestError::BadAttachmentSize(1))
        );
        assert_eq!(
            manifest(vec![meta(1, Photo, MAX_PHOTO_BYTES + 1)]).validate(),
            Err(ManifestError::BadAttachmentSize(1))
        );
        let too_many: Vec<_> = (0..=MAX_PHOTOS_PER_NOTE as u16)
            .map(|i| meta(i, Photo, 1))
            .collect();
        assert_eq!(
            manifest(too_many).validate(),
            Err(ManifestError::TooManyPhotos)
        );
        let mut bad_mime = meta(1, Photo, 1);
        bad_mime.mime = "image/jpeg\n".into();
        assert_eq!(
            manifest(vec![bad_mime]).validate(),
            Err(ManifestError::BadMime(1))
        );
    }

    #[test]
    fn worst_case_frames_fit_the_64_kib_bluetooth_frame() {
        const CAP: usize = 64 * 1024;
        let mut all: Vec<AttachmentMeta> = (0..=MAX_PHOTOS_PER_NOTE as u16)
            .map(|i| AttachmentMeta {
                mime: "m".repeat(MAX_MIME_LEN),
                duration_ms: Some(u32::MAX),
                ..meta(i, AttachmentKind::Photo, MAX_PHOTO_BYTES)
            })
            .collect();
        all[0].kind = AttachmentKind::VoiceRecording;
        let manifest = NoteManifest {
            text: Some("é".repeat(MAX_NOTE_TEXT_BYTES / 2)),
            created_at_ms: u64::MAX,
            ..manifest(all)
        };
        assert_eq!(manifest.validate(), Ok(()));
        let offer = Frame::NoteOffer(NoteOffer { manifest });
        let chunk = Frame::NoteChunk(NoteChunk {
            note_id: NoteId([0xff; 16]),
            attachment_id: u16::MAX,
            offset: u64::MAX,
            data: vec![0xff; MAX_NOTE_CHUNK_LEN as usize],
            sha256: [0xff; 32],
        });
        let tts = Frame::NoteTtsChunk(NoteTtsChunk {
            request_id: u32::MAX,
            offset: u64::MAX,
            data: vec![0xff; MAX_NOTE_CHUNK_LEN as usize],
            sha256: [0xff; 32],
        });
        let transcript = Frame::NoteTranscript(NoteTranscript {
            note_id: NoteId([0xff; 16]),
            attachment_id: u16::MAX,
            text: "é".repeat(MAX_TRANSCRIPT_BYTES / 2),
            truncated: true,
            error: None,
        });
        for f in [offer, chunk, tts, transcript] {
            let n = wire::encode_payload(&f).unwrap().len();
            assert!(n < CAP, "{} is {n} bytes", f.wire_name());
        }
    }

    #[test]
    fn existing_frames_keep_their_discriminants_and_new_ones_follow() {
        // postcard writes the variant index first. These pin the pre-notes wire format.
        let first = |f: &Frame| wire::encode_payload(f).unwrap()[0];
        assert_eq!(first(&Frame::Unpair), 8);
        assert_eq!(first(&Frame::NavEnded), 31);
        assert_eq!(
            first(&Frame::TetherState(crate::TetherState {
                on: false,
                kind: String::new(),
            })),
            75
        );
        assert_eq!(
            first(&Frame::NoteAck(NoteAck {
                note_id: NoteId([0; 16]),
                manifest_sha256: [0; 32],
            })),
            83
        );
    }

    #[test]
    fn every_note_frame_roundtrips() {
        let id = NoteId([9; 16]);
        let frames = vec![
            Frame::NoteOffer(NoteOffer {
                manifest: manifest(vec![meta(0, AttachmentKind::VoiceRecording, 3)]),
            }),
            Frame::NoteOfferReply(NoteOfferReply {
                note_id: id,
                status: NoteOfferStatus::Accepted {
                    progress: vec![AttachmentProgress {
                        attachment_id: 0,
                        next_offset: 2,
                        verified: false,
                    }],
                },
            }),
            Frame::NoteOfferReply(NoteOfferReply {
                note_id: id,
                status: NoteOfferStatus::Deferred {
                    reason: NoteDeferReason::NeedsFasterLink { bytes: 9 },
                },
            }),
            Frame::NoteChunkRequest(NoteChunkRequest {
                note_id: id,
                attachment_id: 0,
                offset: 1,
                max_len: 2,
            }),
            Frame::NoteChunk(NoteChunk {
                note_id: id,
                attachment_id: 0,
                offset: 1,
                data: vec![1, 2],
                sha256: [3; 32],
            }),
            Frame::NoteChunkReject(NoteChunkReject {
                note_id: id,
                attachment_id: 0,
                offset: 1,
                reason: NoteChunkRejectReason::Corrupt,
            }),
            Frame::NoteChunkUnavailable(NoteChunkUnavailable {
                note_id: id,
                attachment_id: 0,
                offset: 1,
                reason: NoteUnavailableReason::SourceError,
            }),
            Frame::NoteAttachmentAck(NoteAttachmentAck {
                note_id: id,
                attachment_id: 0,
                sha256: [3; 32],
                verified: true,
            }),
            Frame::NoteAck(NoteAck {
                note_id: id,
                manifest_sha256: [4; 32],
            }),
            Frame::NoteSyncStatus(NoteSyncStatus {
                pending_notes: 1,
                pending_bytes: 2,
                sending: true,
            }),
            Frame::NoteTranscriptRequest(NoteTranscriptRequest {
                note_id: id,
                attachment_id: 0,
            }),
            Frame::NoteTranscript(NoteTranscript {
                note_id: id,
                attachment_id: 0,
                text: "ciao".into(),
                truncated: false,
                error: Some(NoteTranscriptError::EngineFailed("x".into())),
            }),
            Frame::NoteTranscriptAck(NoteTranscriptAck {
                note_id: id,
                attachment_id: 0,
            }),
            Frame::NoteTtsRequest(NoteTtsRequest {
                request_id: 1,
                note_id: id,
                text: None,
                locale: Locale::Fr,
            }),
            Frame::NoteTtsReady(NoteTtsReady {
                request_id: 1,
                note_id: id,
                sample_rate: 16000,
                total_len: 2,
                sha256: [5; 32],
            }),
            Frame::NoteTtsFailed(NoteTtsFailed {
                request_id: 1,
                note_id: id,
                reason: "no".into(),
            }),
            Frame::NoteTtsChunkRequest(NoteTtsChunkRequest {
                request_id: 1,
                offset: 0,
                max_len: 2,
            }),
            Frame::NoteTtsChunk(NoteTtsChunk {
                request_id: 1,
                offset: 0,
                data: vec![1, 2],
                sha256: [6; 32],
            }),
            Frame::NoteTtsRelease(NoteTtsRelease { request_id: 1 }),
        ];
        for f in frames {
            let bytes = wire::encode_payload(&f).unwrap();
            assert_eq!(wire::decode_payload(&bytes).unwrap(), f);
        }
    }
}
