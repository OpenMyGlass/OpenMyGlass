//! Protocol-level errors (serialization / framing).

use thiserror::Error;

/// Errors from encoding/decoding frames on the wire.
#[derive(Debug, Error)]
pub enum WireError {
    /// postcard (de)serialization failed.
    #[error("serialization error: {0}")]
    Serde(#[from] postcard::Error),

    /// A frame's declared length exceeds [`crate::wire::MAX_FRAME_LEN`].
    #[error("frame too large: {0} bytes (max {max})", max = crate::wire::MAX_FRAME_LEN)]
    FrameTooLarge(usize),

    /// A [`crate::SealedFrame`] did not decrypt, or a frame arrived unencrypted when only sealed
    /// frames are valid. The link must be closed.
    #[error("rejected on the encrypted link: {0}")]
    Rejected(String),

    /// A sealed frame's envelope has bytes after the ciphertext.
    #[error("bytes after a sealed frame")]
    TrailingBytes,

    /// A newer peer sent a [`crate::Frame`] variant this build does not know.
    /// Callers should log and skip rather than abort the session.
    #[error("unknown frame variant from a newer peer")]
    UnknownVariant,
}
