//! Power-aware transport selection and fallback.

use std::sync::Arc;

use tracing::{debug, warn};

use crate::{BoxStream, Link, Transport, TransportError};

/// Which radio a transport uses. Ordered conceptually from cheapest-to-run to most
/// expensive when nothing is already up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    /// Bluetooth Low Energy (not implemented yet; ranked for the policy).
    Ble,
    /// Bluetooth Classic RFCOMM — the normal link.
    BtRfcomm,
    /// Wi-Fi / TCP — fallback and high-bandwidth path.
    WifiTcp,
}

/// Which radios are currently powered. Fed by the energy-policy / radio-PM layer
/// (and, on the Glass side, by glassd's radio state).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RadioSnapshot {
    /// Wi-Fi is already associated (e.g. for internet).
    pub wifi_up: bool,
    /// Bluetooth is already on (e.g. for A2DP/HFP).
    pub bt_up: bool,
    /// BLE is already advertising/connected.
    pub ble_up: bool,
}

/// Lower rank = preferred. Returns `None` for a transport that must not be used in this
/// situation (currently never — all are usable, just ranked).
///
/// Policy (see crate docs):
/// 1. Reuse an already-up radio → rank ~0.
/// 2. Need bandwidth → Wi-Fi/TCP wins.
/// 3. Otherwise prefer BLE < RFCOMM < Wi-Fi (marginal power to bring up).
pub fn rank(kind: TransportKind, radios: &RadioSnapshot, need_bandwidth: bool) -> u32 {
    // 2. Bandwidth dominates: Bluetooth cannot serve a big transfer, so Wi-Fi wins even
    //    over an already-up BT. An already-up Wi-Fi is the ideal (zero marginal cost).
    if need_bandwidth {
        return match kind {
            TransportKind::WifiTcp => {
                if radios.wifi_up {
                    0
                } else {
                    5
                }
            }
            TransportKind::BtRfcomm => 6,
            TransportKind::Ble => 7,
        };
    }

    // 1. Low-bandwidth, reuse a radio already on: near-zero marginal cost.
    let already_up = match kind {
        TransportKind::WifiTcp => radios.wifi_up,
        TransportKind::BtRfcomm => radios.bt_up,
        TransportKind::Ble => radios.ble_up || radios.bt_up,
    };
    if already_up {
        return match kind {
            TransportKind::Ble => 0,
            TransportKind::BtRfcomm => 1,
            TransportKind::WifiTcp => 2,
        };
    }

    // 3. Low-bandwidth, nothing up: cheapest-to-bring-up first.
    match kind {
        TransportKind::Ble => 10,
        TransportKind::BtRfcomm => 11,
        TransportKind::WifiTcp => 12,
    }
}

/// Holds the candidate transports and picks among them.
pub struct TransportManager {
    transports: Vec<Arc<dyn Transport>>,
}

impl TransportManager {
    /// New manager over a set of candidate transports.
    pub fn new(transports: Vec<Arc<dyn Transport>>) -> Self {
        TransportManager { transports }
    }

    /// Order the transports best-first for the current radio state and bandwidth need.
    pub fn order(&self, radios: &RadioSnapshot, need_bandwidth: bool) -> Vec<Arc<dyn Transport>> {
        let mut ranked: Vec<_> = self
            .transports
            .iter()
            .cloned()
            .map(|t| {
                let r = rank(t.kind(), radios, need_bandwidth);
                (r, t)
            })
            .collect();
        ranked.sort_by_key(|(r, _)| *r);
        ranked.into_iter().map(|(_, t)| t).collect()
    }

    /// Dial the best available transport, falling back through the ranked list.
    pub async fn dial_best(
        &self,
        radios: &RadioSnapshot,
        need_bandwidth: bool,
    ) -> Result<(TransportKind, Link<BoxStream>), TransportError> {
        for t in self.order(radios, need_bandwidth) {
            if !t.available().await {
                debug!(kind = ?t.kind(), "transport not available, skipping");
                continue;
            }
            match t.dial().await {
                Ok(link) => {
                    debug!(kind = ?t.kind(), "dialed");
                    return Ok((t.kind(), link));
                }
                Err(e) => {
                    warn!(kind = ?t.kind(), error = %e, "dial failed, falling back");
                }
            }
        }
        Err(TransportError::NoTransport)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn best(radios: RadioSnapshot, need_bw: bool) -> TransportKind {
        let kinds = [
            TransportKind::Ble,
            TransportKind::BtRfcomm,
            TransportKind::WifiTcp,
        ];
        *kinds
            .iter()
            .min_by_key(|k| rank(**k, &radios, need_bw))
            .unwrap()
    }

    #[test]
    fn nothing_up_low_bandwidth_prefers_ble_then_bt() {
        let r = RadioSnapshot::default();
        assert_eq!(best(r, false), TransportKind::Ble);
        // Without BLE implemented, RFCOMM outranks Wi-Fi.
        assert!(rank(TransportKind::BtRfcomm, &r, false) < rank(TransportKind::WifiTcp, &r, false));
    }

    #[test]
    fn nothing_up_high_bandwidth_prefers_wifi() {
        let r = RadioSnapshot::default();
        assert_eq!(best(r, true), TransportKind::WifiTcp);
    }

    #[test]
    fn reuse_wifi_when_already_up() {
        let r = RadioSnapshot {
            wifi_up: true,
            ..Default::default()
        };
        assert_eq!(best(r, false), TransportKind::WifiTcp);
    }

    #[test]
    fn reuse_bt_when_already_up_low_bandwidth() {
        let r = RadioSnapshot {
            bt_up: true,
            ..Default::default()
        };
        // BT reused beats bringing up Wi-Fi.
        assert!(rank(TransportKind::BtRfcomm, &r, false) < rank(TransportKind::WifiTcp, &r, false));
    }

    #[test]
    fn bt_up_but_bandwidth_needed_still_wifi() {
        // Both BT up and a big transfer wanted: Wi-Fi (not yet up) still wins for bandwidth.
        let r = RadioSnapshot {
            bt_up: true,
            ..Default::default()
        };
        assert_eq!(best(r, true), TransportKind::WifiTcp);
    }

    #[test]
    fn wifi_already_up_and_bandwidth_is_cheapest() {
        let r = RadioSnapshot {
            wifi_up: true,
            ..Default::default()
        };
        assert_eq!(rank(TransportKind::WifiTcp, &r, true), 0);
    }
}
