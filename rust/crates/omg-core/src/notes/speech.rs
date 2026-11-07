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

/// Cuts `text` into pieces of at most `max_bytes` for an engine with a limit on its input,
/// after a sentence if one ends in the piece, else after a word. Nothing but the white space
/// around a cut is lost.
pub fn split_for_speech(text: &str, max_bytes: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        if rest.len() <= max_bytes {
            pieces.push(rest);
            break;
        }
        let mut end = max_bytes.max(1);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        // Not even one character fits: it goes whole, or this would never finish.
        if end == 0 {
            end = rest.chars().next().map_or(1, char::len_utf8);
        }
        let window = &rest[..end];
        let cut = window
            .rfind(['.', '!', '?', '\n'])
            .or_else(|| window.rfind(' '))
            .map_or(end, |at| at + 1);
        let piece = rest[..cut].trim();
        if !piece.is_empty() {
            pieces.push(piece);
        }
        rest = rest[cut..].trim_start();
    }
    pieces
}

/// The header of a WAV file with `data_len` bytes of [`SynthesizedAudio::pcm`] behind it, for a
/// phone's media player.
pub fn wav_header(sample_rate: u32, data_len: u32) -> [u8; 44] {
    let mut wav = [0u8; 44];
    wav[..4].copy_from_slice(b"RIFF");
    wav[4..8].copy_from_slice(&data_len.saturating_add(36).to_le_bytes());
    wav[8..16].copy_from_slice(b"WAVEfmt ");
    wav[16..20].copy_from_slice(&16u32.to_le_bytes());
    wav[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    wav[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    wav[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    wav[28..32].copy_from_slice(&sample_rate.saturating_mul(2).to_le_bytes());
    wav[32..34].copy_from_slice(&2u16.to_le_bytes()); // bytes per frame
    wav[34..36].copy_from_slice(&16u16.to_le_bytes());
    wav[36..40].copy_from_slice(b"data");
    wav[40..44].copy_from_slice(&data_len.to_le_bytes());
    wav
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_is_cut_after_sentences_then_words_and_loses_no_words() {
        assert!(split_for_speech("   ", 10).is_empty());
        assert_eq!(split_for_speech(" short ", 10), ["short"]);
        assert_eq!(
            split_for_speech("Uno due. Tre quattro cinque", 12),
            ["Uno due.", "Tre quattro", "cinque"]
        );
        let long = "parola ".repeat(5000);
        let pieces = split_for_speech(&long, 3000);
        assert!(pieces.iter().all(|p| p.len() <= 3000));
        assert_eq!(
            pieces.join(" ").split_whitespace().count(),
            long.split_whitespace().count()
        );
    }

    #[test]
    fn speech_is_never_cut_inside_a_character_and_always_ends() {
        let text = "è".repeat(100);
        for limit in [0, 1, 2, 3, 4, 5, 7] {
            let pieces = split_for_speech(&text, limit);
            assert_eq!(pieces.concat(), text, "limit {limit}");
        }
        let unbroken = "x".repeat(10);
        assert_eq!(split_for_speech(&unbroken, 4), ["xxxx", "xxxx", "xx"]);
    }

    #[test]
    fn a_four_byte_character_is_kept_whole_at_any_limit() {
        let text = "a😀b😀😀c";
        for limit in 0..=8 {
            let pieces = split_for_speech(text, limit);
            assert_eq!(pieces.concat(), text, "limit {limit}");
        }
        assert_eq!(split_for_speech("😀😀", 5), ["😀", "😀"]);
    }

    #[test]
    fn a_wav_says_what_it_holds() {
        let wav = wav_header(16_000, 4);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 40);
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(wav[28..32].try_into().unwrap()), 32_000);
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 4);
    }
}
