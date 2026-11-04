//! What each side persists about a note. Written as one JSON file per note by
//! [`crate::notes::store`].

use std::collections::{BTreeMap, BTreeSet};

use omg_protocol::{
    AttachmentId, NoteId, NoteManifest, NoteRejectReason, NoteTranscriptError, Sha256Digest,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// On-disk format of the records in this module. A newer format is set aside at load,
/// never overwritten.
pub const RECORD_FORMAT: u32 = 1;

/// A record the store can hold.
pub trait NoteRecord: Serialize + DeserializeOwned {
    /// The note this record is about.
    fn note_id(&self) -> NoteId;
    /// The record's format version.
    fn format(&self) -> u32;
}

/// Why a note on the Glass will not be offered again until someone looks at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockReason {
    /// The companion refused the note.
    Rejected(NoteRejectReason),
    /// A file is unreadable or no longer matches the manifest.
    SourceCorrupt,
}

/// Where a note on the Glass stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutgoingState {
    /// Waiting to reach the companion.
    Queued,
    /// The companion verified everything. `delete_pending` records that the delete policy
    /// asked for removal at that moment, so a crash cannot lose the decision and a later
    /// policy change cannot widen it.
    Synced {
        /// Remove the local files.
        delete_pending: bool,
    },
    /// Kept on the Glass, not offered again until retried.
    Blocked(BlockReason),
}

/// A transcript as the Glass received it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceivedTranscript {
    /// The text, possibly cut short.
    pub text: String,
    /// The companion's copy is longer.
    pub truncated: bool,
    /// Set when there is no transcript.
    pub error: Option<NoteTranscriptError>,
}

/// The Glass's record of a sealed note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutgoingNote {
    /// [`RECORD_FORMAT`].
    pub format: u32,
    /// Position in the queue. A local counter, because the Glass clock may be wrong.
    pub seq: u64,
    /// The sealed manifest.
    pub manifest: NoteManifest,
    /// Its digest.
    pub manifest_sha256: Sha256Digest,
    /// Attachments the companion verified, with a digest matching the manifest.
    pub verified: BTreeSet<AttachmentId>,
    /// Where the note stands.
    pub state: OutgoingState,
    /// Transcripts received, by voice attachment.
    pub transcripts: BTreeMap<AttachmentId, ReceivedTranscript>,
}

impl NoteRecord for OutgoingNote {
    fn note_id(&self) -> NoteId {
        self.manifest.id
    }
    fn format(&self) -> u32 {
        self.format
    }
}

/// How far the companion has one attachment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncomingAttachment {
    /// Bytes known durable. A partial file longer than this is cut back on resume.
    pub committed_offset: u64,
    /// The file is in its final place and its SHA-256 matched.
    pub verified: bool,
    /// Whole-file checksum failures since the last give-up.
    pub hash_failures: u32,
}

/// A transcript as the companion holds it: always the full text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTranscript {
    /// The full text. Empty on error.
    pub text: String,
    /// Set when there is no transcript.
    pub error: Option<NoteTranscriptError>,
}

/// One transcription on the companion.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptRecord {
    /// The result, once the engine has run.
    pub result: Option<StoredTranscript>,
    /// The Glass confirmed it has the result.
    pub acked: bool,
}

/// The companion's record of a note it has been offered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncomingNote {
    /// [`RECORD_FORMAT`].
    pub format: u32,
    /// The manifest as first offered. Later offers must match it.
    pub manifest: NoteManifest,
    /// Its digest.
    pub manifest_sha256: Sha256Digest,
    /// Progress by attachment.
    pub attachments: BTreeMap<AttachmentId, IncomingAttachment>,
    /// Every attachment is durable and verified.
    pub complete: bool,
    /// Transcriptions requested, by voice attachment.
    pub transcripts: BTreeMap<AttachmentId, TranscriptRecord>,
    /// When first offered, companion clock, Unix ms.
    pub received_at_ms: u64,
}

impl NoteRecord for IncomingNote {
    fn note_id(&self) -> NoteId {
        self.manifest.id
    }
    fn format(&self) -> u32 {
        self.format
    }
}
