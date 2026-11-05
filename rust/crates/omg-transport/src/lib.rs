//! OpenMyGlass link transport.
//!
//! The link carries [`omg_protocol::Frame`]s. This crate provides:
//!
//! * [`codec::FrameCodec`] — length-delimited framing over any byte stream, plus
//!   [`Link`], a [`Frame`](omg_protocol::Frame) sink/stream.
//! * [`Transport`] — a trait so the core does not care which radio it runs over.
//! * [`TcpTransport`] — the Wi-Fi/TCP path (works today, and the fallback for a PC
//!   without Bluetooth).
//! * [`BtRfcommTransport`] — the Bluetooth Classic RFCOMM path (the normal, lower-power
//!   link). Stubbed behind the trait; a BlueZ implementation lands under the `linux-bt`
//!   feature.
//! * [`TransportManager`] + [`rank`] — **power-aware selection**: pick the lowest
//!   marginal-power transport available right now, and fall back.
//!
//! ## Power-aware selection (owner decision 2026-11-01)
//!
//! The link has no fixed primary. The manager ranks transports by *marginal* energy
//! cost given which radios are already up:
//!
//! 1. **Reuse a radio already on** (≈ zero marginal cost): if Wi-Fi is already up for
//!    internet, run over TCP; if BT is already up for A2DP/HFP, run over RFCOMM.
//! 2. **Otherwise prefer Bluetooth** (BLE then RFCOMM) — cheapest to bring up for the
//!    low-bandwidth, bursty companion traffic; staying Wi-Fi-associated costs more.
//! 3. **Fall back to Wi-Fi/TCP** when BT is unavailable (PC without BT) **or** a
//!    high-bandwidth transfer is needed (media sync / OTA — Wi-Fi wins there).
//!
//! See [`rank`] for the exact ranking and [`TransportManager::dial_best`] for the
//! try-then-fall-back behaviour.

pub mod bt;
pub mod codec;
pub mod manager;
pub mod tcp;

pub use bt::BtRfcommTransport;
pub use codec::{FrameCipher, FrameCodec, Link};
pub use manager::{rank, RadioSnapshot, TransportKind, TransportManager};
pub use tcp::{TcpListenerTransport, TcpTransport};

use thiserror::Error;

/// A bidirectional byte stream a [`Transport`] hands to the codec.
pub trait Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Stream for T {}

/// A boxed byte stream, so transports of different concrete types are interchangeable.
pub type BoxStream = Box<dyn Stream>;

/// Errors from establishing or running a transport.
#[derive(Debug, Error)]
pub enum TransportError {
    /// Underlying I/O error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// Protocol (de)serialization error on the framed link.
    #[error("wire: {0}")]
    Wire(#[from] omg_protocol::WireError),

    /// This transport is not available on this platform/build (e.g. the BT stub).
    #[error("transport unavailable: {0}")]
    Unavailable(String),

    /// No transport could be dialed.
    #[error("no transport available")]
    NoTransport,
}

/// A way to open the companion link. Implementations are cheap handles; dialing does
/// the real work.
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// Which radio this transport uses.
    fn kind(&self) -> TransportKind;

    /// Whether this transport looks usable right now (radio present / peer reachable).
    /// Cheap and best-effort; [`dial`](Transport::dial) is authoritative.
    async fn available(&self) -> bool;

    /// Open an outbound link to the peer (companion dials the Glass).
    async fn dial(&self) -> Result<Link<BoxStream>, TransportError>;
}
