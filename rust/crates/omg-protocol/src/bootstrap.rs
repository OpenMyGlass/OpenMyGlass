//! Wi-Fi QR bootstrap payload.
//!
//! The chicken-and-egg problem: the companion link needs connectivity, but configuring
//! Wi-Fi normally needs the link. The bootstrap breaks it **offline**: the companion
//! generates a QR code, the Glass **camera scans it** (no Google servers), and glassd
//! joins the network and then finds the companion.
//!
//! This type is defined in the protocol crate so the generator (`omg-qr`, companion
//! side) and the decoder (glassd on the Glass) share one definition and cannot drift.
//!
//! ## Encodings
//!
//! * [`WifiBootstrap::encode`] → `OMG1:<base64url(postcard)>` — the OpenMyGlass format.
//!   Carries the extra fields (companion address, mDNS service, one-time pair token)
//!   that a plain Wi-Fi QR cannot. **glassd decodes this.**
//! * [`WifiBootstrap::to_wifi_uri`] → the standard `WIFI:S:…;T:…;P:…;H:…;;` string that
//!   generic phone cameras understand. Join-only, no OpenMyGlass extras. Provided for
//!   convenience/debugging.

use serde::{Deserialize, Serialize};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;

/// Current bootstrap payload version.
pub const BOOTSTRAP_VERSION: u8 = 1;

/// The `OMG1:` scheme prefix.
pub const OMG_PREFIX: &str = "OMG1:";

/// Wi-Fi authentication type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WifiAuth {
    /// Open network, no key.
    Open,
    /// WEP (legacy).
    Wep,
    /// WPA/WPA2-PSK.
    WpaPsk,
    /// WPA3-SAE.
    Wpa3Sae,
}

impl WifiAuth {
    /// The token used in a standard `WIFI:` URI (`T:` field).
    fn wifi_uri_token(self) -> &'static str {
        match self {
            WifiAuth::Open => "nopass",
            WifiAuth::Wep => "WEP",
            WifiAuth::WpaPsk => "WPA",
            WifiAuth::Wpa3Sae => "SAE",
        }
    }
}

/// Everything the Glass needs to join Wi-Fi and find its companion, encoded into a QR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiBootstrap {
    /// Payload version ([`BOOTSTRAP_VERSION`]).
    pub version: u8,
    /// Network SSID.
    pub ssid: String,
    /// Authentication type.
    pub auth: WifiAuth,
    /// Pre-shared key (None for open networks).
    pub psk: Option<String>,
    /// Whether the SSID is hidden.
    pub hidden: bool,
    /// Direct companion address hint `ip:port` (skips discovery when the IP is known).
    pub companion_addr: Option<String>,
    /// mDNS service to discover the companion by (its IP changes), e.g.
    /// `_openmyglass._tcp.local`.
    pub mdns_service: Option<String>,
    /// One-time token binding the FIRST pairing to the companion that made this QR.
    pub pair_token: Option<String>,
}

impl WifiBootstrap {
    /// Build an open-network bootstrap.
    pub fn open(ssid: impl Into<String>) -> Self {
        WifiBootstrap {
            version: BOOTSTRAP_VERSION,
            ssid: ssid.into(),
            auth: WifiAuth::Open,
            psk: None,
            hidden: false,
            companion_addr: None,
            mdns_service: None,
            pair_token: None,
        }
    }

    /// Build a WPA/WPA2-PSK bootstrap.
    pub fn wpa(ssid: impl Into<String>, psk: impl Into<String>) -> Self {
        WifiBootstrap {
            version: BOOTSTRAP_VERSION,
            ssid: ssid.into(),
            auth: WifiAuth::WpaPsk,
            psk: Some(psk.into()),
            hidden: false,
            companion_addr: None,
            mdns_service: None,
            pair_token: None,
        }
    }

    /// Encode to the OpenMyGlass `OMG1:` QR string (what glassd decodes).
    pub fn encode(&self) -> String {
        let bytes = postcard::to_allocvec(self).expect("WifiBootstrap is always serializable");
        format!("{OMG_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
    }

    /// Decode an `OMG1:` QR string.
    pub fn decode(s: &str) -> Result<Self, BootstrapError> {
        let b64 = s
            .strip_prefix(OMG_PREFIX)
            .ok_or(BootstrapError::BadPrefix)?;
        let bytes = URL_SAFE_NO_PAD
            .decode(b64.as_bytes())
            .map_err(|_| BootstrapError::Base64)?;
        let v: WifiBootstrap = postcard::from_bytes(&bytes).map_err(|_| BootstrapError::Decode)?;
        if v.version != BOOTSTRAP_VERSION {
            return Err(BootstrapError::Version(v.version));
        }
        Ok(v)
    }

    /// Standard `WIFI:` URI for generic scanners (no OpenMyGlass extras). Special
    /// characters in the SSID/PSK are backslash-escaped per the de-facto format.
    pub fn to_wifi_uri(&self) -> String {
        let s = escape_wifi(&self.ssid);
        let t = self.auth.wifi_uri_token();
        let p = self.psk.as_deref().map(escape_wifi).unwrap_or_default();
        let h = if self.hidden { "true" } else { "" };
        format!("WIFI:S:{s};T:{t};P:{p};H:{h};;")
    }
}

/// Errors decoding a bootstrap string.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum BootstrapError {
    /// Missing the `OMG1:` prefix.
    #[error("not an OMG1 bootstrap string")]
    BadPrefix,
    /// Base64 decode failed.
    #[error("invalid base64")]
    Base64,
    /// postcard decode failed.
    #[error("invalid payload")]
    Decode,
    /// Unsupported payload version.
    #[error("unsupported bootstrap version {0}")]
    Version(u8),
}

/// Escape `\`, `;`, `,`, `:`, `"` for the `WIFI:` URI format.
fn escape_wifi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | ';' | ',' | ':' | '"') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omg_roundtrip_full() {
        let mut b = WifiBootstrap::wpa("Casa-Giorgio", "s3cr3t-pass");
        b.hidden = true;
        b.companion_addr = Some("10.10.10.50:7700".into());
        b.mdns_service = Some("_openmyglass._tcp.local".into());
        b.pair_token = Some("tok-abc123".into());

        let s = b.encode();
        assert!(s.starts_with(OMG_PREFIX));
        let back = WifiBootstrap::decode(&s).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn rejects_non_omg() {
        assert_eq!(
            WifiBootstrap::decode("WIFI:S:x;;"),
            Err(BootstrapError::BadPrefix)
        );
    }

    #[test]
    fn standard_wifi_uri_escapes() {
        let b = WifiBootstrap::wpa("My;Net", "a:b\\c");
        assert_eq!(b.to_wifi_uri(), "WIFI:S:My\\;Net;T:WPA;P:a\\:b\\\\c;H:;;");
    }

    #[test]
    fn open_network_uri() {
        let b = WifiBootstrap::open("Guest");
        assert_eq!(b.to_wifi_uri(), "WIFI:S:Guest;T:nopass;P:;H:;;");
    }
}
