//! Bluetooth Classic RFCOMM transport — the normal, lower-power companion link.
//!
//! Status: **stub behind the [`Transport`] trait**. BT hardware integration is
//! platform-specific (Linux BlueZ on a PC, Android `BluetoothSocket` on a phone), so the
//! core only ever sees [`Transport`]; this file keeps the companion link radio-agnostic.
//!
//! ## Why BT is the default link (owner decision 2026-11-01)
//!
//! * BT is needed anyway for A2DP headphones and HFP calls, so the companion link rides
//!   the same radio — one radio on, not two.
//! * For the low-bandwidth, bursty companion traffic (notifications, commands, dictation
//!   text, cards) BT is far cheaper than staying Wi-Fi-associated.
//! * The Glass keeps Wi-Fi **off** by default; Wi-Fi is brought up on demand only.
//!
//! ## Implementing it (the `linux-bt` feature, TODO)
//!
//! On Linux, use `bluer` (BlueZ D-Bus): open an RFCOMM socket to the bonded Glass on the
//! OpenMyGlass service UUID [`OMG_COMPANION_RFCOMM_UUID`], then wrap the socket in
//! [`Link`](crate::Link). Persistent bonding keys live in `/var/lib/bluetooth`, which is
//! what makes auto-reconnect work (see the persistent-pairing decision). glassd is the
//! RFCOMM *server* advertising this UUID via SDP; the companion is the client.
//!
//! On Android the frontend provides the socket via JNI (`createRfcommSocketToService
//! Record`) and hands its fd to this crate.

use crate::{BoxStream, Link, Transport, TransportError, TransportKind};

/// OpenMyGlass's own RFCOMM service UUID (clean-room; NOT Google's companion UUID).
/// glassd advertises this over SDP; the companion connects to it.
pub const OMG_COMPANION_RFCOMM_UUID: &str = "6f6d672d-636f-6d70-616e-696f6e2d7631";

/// A Bluetooth address (big-endian `XX:XX:XX:XX:XX:XX`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtAddr(pub String);

/// RFCOMM transport to a bonded Glass. Dialing is a no-op stub until `linux-bt` lands.
#[derive(Debug, Clone)]
pub struct BtRfcommTransport {
    addr: BtAddr,
    uuid: String,
}

impl BtRfcommTransport {
    /// New RFCOMM transport to the bonded Glass at `addr`, using the OpenMyGlass UUID.
    pub fn new(addr: BtAddr) -> Self {
        BtRfcommTransport {
            addr,
            uuid: OMG_COMPANION_RFCOMM_UUID.to_string(),
        }
    }

    /// The target Bluetooth address.
    pub fn addr(&self) -> &BtAddr {
        &self.addr
    }

    /// The RFCOMM service UUID this transport dials.
    pub fn uuid(&self) -> &str {
        &self.uuid
    }
}

#[async_trait::async_trait]
impl Transport for BtRfcommTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::BtRfcomm
    }

    async fn available(&self) -> bool {
        // No BlueZ integration yet, so the BT path is never selectable; the manager
        // falls back to TCP. Flip this on once `linux-bt` implements dial().
        false
    }

    async fn dial(&self) -> Result<Link<BoxStream>, TransportError> {
        #[cfg(feature = "linux-bt")]
        {
            // TODO(linux-bt): open an RFCOMM socket to self.addr on self.uuid via bluer
            // and return Link::boxed(socket).
            Err(TransportError::Unavailable(
                "linux-bt RFCOMM dial not implemented yet".into(),
            ))
        }
        #[cfg(not(feature = "linux-bt"))]
        {
            Err(TransportError::Unavailable(format!(
                "Bluetooth RFCOMM not built in (addr={}, uuid={}); rebuild with --features linux-bt",
                self.addr.0, self.uuid
            )))
        }
    }
}
