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

    /// A newer peer sent a [`crate::Frame`] variant this build does not know.
    /// Callers should log and skip rather than abort the session.
    #[error("unknown frame variant from a newer peer")]
    UnknownVariant,
}
