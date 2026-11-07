//! Notes with attachments and deferred sync.
//!
//! A note is sealed on the Glass and stays there until the companion holds a verified,
//! durable copy. Layers, from pure to effectful:
//!
//! * [`sender`] and [`receiver`]: pure state machines for the Glass and the companion.
//!   No I/O, no clock, no randomness: they take events plus `now_ms` and return effects
//!   to carry out, in order.
//! * [`store`]: the crash-safe on-disk queue.
//! * [`driver`]: applies effects against the store. Synchronous and transport-free, so a
//!   phone app can feed it frames from whatever link it owns.
//! * `link`: an optional async loop over an `omg-transport` link, used by the simulated
//!   Glass and the CLI. Needs the `async-link` feature.
//! * [`speech`]: traits for the speech engines the phone apps implement.
//!
//! The protocol is in [`omg_protocol::notes`]; the design in `docs/companion-notes-sync.md`.

pub mod driver;
#[cfg(feature = "async-link")]
pub mod link;
pub mod receiver;
pub mod record;
pub mod sender;
pub mod speech;
pub mod store;

pub use driver::{
    AttachmentSource, CompanionNotes, GlassNotes, NoteDraft, NoteSealer, SpeechJob, SpeechOutcome,
};
#[cfg(feature = "async-link")]
pub use link::{run_companion, CompanionRunOptions, CompanionUpdate};
pub use receiver::{NoteReceiver, ReceiverEffect, ReceiverEvent};
pub use record::{IncomingNote, OutgoingNote, OutgoingState};
pub use sender::{DeletePolicy, NoteSender, SenderConfig, SenderEffect, SenderEvent};
pub use speech::{
    NoEngine, SpeechError, SpeechToText, SynthesizeRequest, SynthesizedAudio, TextToSpeech,
    TranscribeRequest,
};
pub use store::NoteStore;

use omg_protocol::{Frame, NoteManifest, Sha256Digest, MAX_NOTE_CHUNK_LEN};
use sha2::{Digest, Sha256};

/// Attachments over this size wait for a faster link on a `constrained` link: about 90 s at the
/// 3 kB/s BLE measured. The owner's decision: see `docs/companion-notes-sync.md`.
pub const AUTO_SYNC_LIMIT_CONSTRAINED: u64 = 256 * 1024;
/// The same on a `moderate` link.
pub const AUTO_SYNC_LIMIT_MODERATE: u64 = 64 * 1024 * 1024;

/// Whether a frame belongs to note sync.
pub fn is_note_frame(frame: &Frame) -> bool {
    frame.wire_name().starts_with("Note")
}

/// How to pull over the current link. Plain data supplied by whoever owns the link, so
/// the core assumes no particular radio. The numbers are starting values, not yet
/// measured on hardware; see `docs/companion-notes-sync.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkProfile {
    /// Bytes per chunk, clamped to `1..=`[`MAX_NOTE_CHUNK_LEN`]. Also bounds how long
    /// one chunk holds the link ahead of navigation and notification frames.
    pub chunk_len: u32,
    /// Chunks outstanding at once (at least 1).
    pub window: u32,
    /// An attachment over this size is left for a faster link instead of syncing
    /// automatically. `None` means no limit.
    pub max_auto_attachment_bytes: Option<u64>,
}

impl LinkProfile {
    /// A low-throughput link such as Bluetooth LE, the everyday link. A frame sent during a
    /// transfer waits behind about a second of chunks at the 3 kB/s measured on the Glass.
    pub const fn constrained() -> Self {
        LinkProfile {
            chunk_len: 1024,
            window: 2,
            max_auto_attachment_bytes: Some(AUTO_SYNC_LIMIT_CONSTRAINED),
        }
    }

    /// A moderate link such as Bluetooth Classic RFCOMM. Two 16 KiB chunks in flight keep
    /// other frames waiting for a fraction of a second at worst.
    pub const fn moderate() -> Self {
        LinkProfile {
            chunk_len: 16 * 1024,
            window: 2,
            max_auto_attachment_bytes: Some(AUTO_SYNC_LIMIT_MODERATE),
        }
    }

    /// A fast link such as Wi-Fi.
    pub const fn fast() -> Self {
        LinkProfile {
            chunk_len: MAX_NOTE_CHUNK_LEN,
            window: 8,
            max_auto_attachment_bytes: None,
        }
    }

    /// This profile with its values forced into the protocol's range.
    pub fn normalized(self) -> Self {
        LinkProfile {
            chunk_len: self.chunk_len.clamp(1, MAX_NOTE_CHUNK_LEN),
            window: self.window.max(1),
            ..self
        }
    }
}

/// Retry timing for both state machines. All times are in milliseconds of the `now_ms`
/// passed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Silence from the peer before an offer or a chunk request is repeated.
    pub response_timeout_ms: u64,
    /// First delay after the companion defers a note. Doubles per attempt.
    pub backoff_base_ms: u64,
    /// Longest delay.
    pub backoff_max_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            response_timeout_ms: 30_000,
            backoff_base_ms: 5_000,
            backoff_max_ms: 3_600_000,
        }
    }
}

impl RetryPolicy {
    /// The delay before attempt number `attempt` (0-based).
    pub fn backoff(&self, attempt: u32) -> u64 {
        self.backoff_base_ms
            .saturating_mul(1u64 << attempt.min(32))
            .min(self.backoff_max_ms)
    }
}

/// SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> Sha256Digest {
    Sha256::digest(data).into()
}

/// The digest acknowledgements carry: SHA-256 of the manifest's canonical bytes.
pub fn manifest_digest(manifest: &NoteManifest) -> Sha256Digest {
    sha256(&manifest.canonical_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_frames_are_told_apart_from_notification_frames() {
        assert!(is_note_frame(&Frame::NoteSyncStatus(
            omg_protocol::NoteSyncStatus {
                pending_notes: 0,
                pending_bytes: 0,
                sending: false,
            }
        )));
        assert!(!is_note_frame(&Frame::NotificationRemoved(
            omg_protocol::NotificationRemoved { key: "k".into() }
        )));
        assert!(!is_note_frame(&Frame::Ping(omg_protocol::Ping {
            token: 0,
            unix_ms: 0
        })));
    }

    #[test]
    fn backoff_doubles_and_saturates() {
        let p = RetryPolicy::default();
        assert_eq!(p.backoff(0), 5_000);
        assert_eq!(p.backoff(1), 10_000);
        assert_eq!(p.backoff(1000), p.backoff_max_ms);
    }

    #[test]
    fn profiles_stay_in_the_protocol_range() {
        let wild = LinkProfile {
            chunk_len: u32::MAX,
            window: 0,
            max_auto_attachment_bytes: None,
        }
        .normalized();
        assert_eq!(wild.chunk_len, MAX_NOTE_CHUNK_LEN);
        assert_eq!(wild.window, 1);
        for p in [
            LinkProfile::constrained(),
            LinkProfile::moderate(),
            LinkProfile::fast(),
        ] {
            assert_eq!(p, p.normalized());
        }
    }
}
