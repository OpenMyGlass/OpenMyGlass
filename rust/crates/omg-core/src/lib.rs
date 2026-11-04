//! OpenMyGlass companion core.
//!
//! Platform-independent logic shared by every frontend (CLI/daemon now; Android/desktop
//! UI later). It owns:
//!
//! * [`identity`] — the long-term ed25519 identity (trust anchor).
//! * [`store`] — persistent state: our identity + the single bonded peer.
//! * [`session`] — the link state machine: Hello, one-time pairing, mutual auth, then
//!   heartbeat / locale / status / notifications.
//! * [`notes`] — notes with attachments and deferred sync: pure state machines for both
//!   sides, a crash-safe queue, and the speech engine traits the phone apps implement.
//! * [`simglass`] — a reusable simulated-Glass peer for tests and the CLI, so the whole
//!   link runs end-to-end without hardware. It mirrors what glassd must do.
//!
//! The backbone (pairing, heartbeat, locale, battery/status, notification forwarding) and
//! note sync are implemented and tested end-to-end. Everything else in [`omg_protocol`] is
//! typed and scaffolded.

pub mod identity;
pub mod notes;
pub mod session;
pub mod simglass;
pub mod store;

pub use identity::Identity;
pub use session::{PairingMode, Session, SessionConfig};
pub use store::{PairingRecord, Store};

use std::time::{SystemTime, UNIX_EPOCH};

use omg_protocol::ErrorMsg;
use thiserror::Error;

/// Milliseconds since the Unix epoch (monotonic enough for tokens/timestamps).
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Errors from the companion core.
#[derive(Debug, Error)]
pub enum CoreError {
    /// Transport/framing error.
    #[error("transport: {0}")]
    Transport(#[from] omg_transport::TransportError),

    /// Filesystem I/O error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// State file could not be read/parsed/written.
    #[error("store: {0}")]
    Store(String),

    /// Identity secret was malformed.
    #[error("invalid identity key")]
    BadIdentity,

    /// The peer is not the bonded one; connection refused.
    #[error("unknown peer: not the bonded companion/glass")]
    UnknownPeer,

    /// No pairing exists and a new one was not permitted.
    #[error("not paired")]
    NotPaired,

    /// Mutual authentication failed (bad signature).
    #[error("authentication failed")]
    AuthFailed,

    /// Protocol versions are incompatible.
    #[error("protocol version mismatch: ours v{ours}, theirs v{theirs}")]
    VersionMismatch {
        /// Our protocol version.
        ours: u16,
        /// The peer's protocol version.
        theirs: u16,
    },

    /// A malformed or out-of-sequence frame.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// The peer sent an [`ErrorMsg`].
    #[error("peer error: {0:?}")]
    PeerError(ErrorMsg),

    /// A note breaks a protocol limit, or could not be sealed.
    #[error("invalid note: {0}")]
    InvalidNote(String),

    /// The link closed unexpectedly mid-handshake.
    #[error("unexpected end of stream")]
    UnexpectedEof,
}
