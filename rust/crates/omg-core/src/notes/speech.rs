//! Speech engines, implemented by the phone apps.
//!
//! Speech-to-text and TTS always run on the companion, never on the Glass: only the
//! companion's driver ([`crate::notes::CompanionNotes`]) ever produces a job for these
//! traits, and the Glass side has no path to them. No engine ships in this crate.
//!
//! The calls are blocking on purpose: it is the simplest shape to implement across a
//! foreign-language boundary, and an app can wrap an asynchronous platform API by waiting
//! on it. They are never called on the thread that services the link.

use std::path::PathBuf;

use omg_protocol::{AttachmentId, Locale, NoteId};
use thiserror::Error;

/// Why an engine produced nothing.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SpeechError {
    /// No engine here, or none for this language or format.
    #[error("no speech engine available")]
    Unavailable,
    /// The engine failed.
    #[error("speech engine failed: {0}")]
    Failed(String),
}

/// Transcribe a recording that the companion holds, verified, in its note store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscribeRequest {
    /// The note.
    pub note_id: NoteId,
    /// The voice attachment.
    pub attachment_id: AttachmentId,
    /// Where the recording is. Read it; do not move or change it.
    pub audio_path: PathBuf,
    /// Its MIME type, as the Glass declared it.
    pub mime: String,
    /// Language of the note.
    pub locale: Locale,
}

/// Speak `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynthesizeRequest {
    /// The note this is for.
    pub note_id: NoteId,
    /// What to say.
    pub text: String,
    /// Language to speak.
    pub locale: Locale,
}

/// Synthesized speech: PCM 16-bit little-endian mono, what the Glass plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynthesizedAudio {
    /// PCM sample rate.
    pub sample_rate: u32,
    /// The samples.
    pub pcm: Vec<u8>,
}

/// A speech-to-text engine.
pub trait SpeechToText: Send + Sync {
    /// Transcribe the recording. Blocking.
    fn transcribe(&self, request: &TranscribeRequest) -> Result<String, SpeechError>;
}

/// A text-to-speech engine.
pub trait TextToSpeech: Send + Sync {
    /// Synthesize the text. Blocking.
    fn synthesize(&self, request: &SynthesizeRequest) -> Result<SynthesizedAudio, SpeechError>;
}

/// The engine for a companion that has none: every request is [`SpeechError::Unavailable`].
#[derive(Debug, Clone, Copy, Default)]
pub struct NoEngine;

impl SpeechToText for NoEngine {
    fn transcribe(&self, _: &TranscribeRequest) -> Result<String, SpeechError> {
        Err(SpeechError::Unavailable)
    }
}

impl TextToSpeech for NoEngine {
    fn synthesize(&self, _: &SynthesizeRequest) -> Result<SynthesizedAudio, SpeechError> {
        Err(SpeechError::Unavailable)
    }
}
