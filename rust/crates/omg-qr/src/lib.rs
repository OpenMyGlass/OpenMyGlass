//! Render a [`WifiBootstrap`] as a scannable QR code, fully **offline**.
//!
//! The companion shows this QR; the Glass camera scans it; glassd decodes the
//! [`WifiBootstrap`] (the `OMG1:` form), joins Wi-Fi and finds the companion. No Google
//! servers are involved.
//!
//! glassd decoder side: scan the camera frame, read the QR text, and call
//! [`omg_protocol::bootstrap::WifiBootstrap::decode`]. A Rust QR *decoder* such as `rqrr`
//! or `quircs` turns the camera image into the string; this crate only *encodes*.

use omg_protocol::bootstrap::WifiBootstrap;
use qrcode::render::svg;
use qrcode::{EcLevel, QrCode};
use thiserror::Error;

/// Errors generating a QR.
#[derive(Debug, Error)]
pub enum QrError {
    /// The payload was too large to fit any QR version.
    #[error("qr encode failed: {0}")]
    Encode(#[from] qrcode::types::QrError),
}

/// A rendered QR plus the exact string it encodes (so glassd authors can see the
/// payload and the companion can log it).
pub struct BootstrapQr {
    /// The `OMG1:` text the QR encodes.
    pub payload: String,
    code: QrCode,
}

impl BootstrapQr {
    /// Build a QR from a [`WifiBootstrap`] using its `OMG1:` encoding.
    pub fn from_bootstrap(b: &WifiBootstrap) -> Result<Self, QrError> {
        let payload = b.encode();
        // Medium error correction: robust to a smudged prism camera without bloating
        // the module count.
        let code = QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::M)?;
        Ok(BootstrapQr { payload, code })
    }

    /// Render as a compact terminal string (two rows of modules per text line).
    pub fn to_terminal(&self) -> String {
        self.code
            .render::<qrcode::render::unicode::Dense1x2>()
            .quiet_zone(true)
            .module_dimensions(1, 1)
            .build()
    }

    /// Render as a self-contained SVG string.
    pub fn to_svg(&self) -> String {
        self.code
            .render::<svg::Color>()
            .min_dimensions(256, 256)
            .quiet_zone(true)
            .build()
    }

    /// QR version width/height in modules (for diagnostics).
    pub fn module_count(&self) -> usize {
        self.code.width()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omg_protocol::bootstrap::WifiBootstrap;

    #[test]
    fn generates_qr_and_payload_roundtrips() {
        let mut b = WifiBootstrap::wpa("Casa-Giorgio", "s3cr3t");
        b.companion_addr = Some("10.10.10.50:7700".into());
        b.pair_token = Some("tok-xyz".into());

        let qr = BootstrapQr::from_bootstrap(&b).unwrap();
        assert!(qr.payload.starts_with("OMG1:"));
        assert!(qr.module_count() >= 21); // smallest QR is 21x21

        // The embedded payload decodes back to the same bootstrap.
        let back = WifiBootstrap::decode(&qr.payload).unwrap();
        assert_eq!(b, back);

        let term = qr.to_terminal();
        assert!(!term.is_empty());
        let svg = qr.to_svg();
        assert!(svg.contains("<svg"));
    }
}
