//! Framing and (de)serialization of a single [`crate::Frame`].
//!
//! On the wire a frame is:
//!
//! ```text
//! +------------------+---------------------------+
//! | u32 big-endian N | postcard(Frame)  (N bytes)|
//! +------------------+---------------------------+
//! ```
//!
//! The length prefix is handled by the transport codec (`omg-transport`). This module
//! only turns a [`crate::Frame`] into/out of its payload bytes, so the schema stays
//! transport-agnostic and glassd can reuse it verbatim.

use crate::{error::WireError, messages::Frame};

/// Hard cap on a single frame's payload. Large media/OTA transfers are chunked well
/// below this; the cap exists to bound memory on the Glass and reject garbage lengths.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

/// Serialize one frame to its postcard payload (no length prefix).
pub fn encode_payload(frame: &Frame) -> Result<Vec<u8>, WireError> {
    let bytes = postcard::to_allocvec(frame)?;
    if bytes.len() > MAX_FRAME_LEN {
        return Err(WireError::FrameTooLarge(bytes.len()));
    }
    Ok(bytes)
}

/// Deserialize one frame from a postcard payload (no length prefix).
pub fn decode_payload(bytes: &[u8]) -> Result<Frame, WireError> {
    if bytes.len() > MAX_FRAME_LEN {
        return Err(WireError::FrameTooLarge(bytes.len()));
    }
    Ok(postcard::from_bytes(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::*;

    #[test]
    fn roundtrip_backbone_frames() {
        let frames = vec![
            Frame::Ping(Ping {
                token: 7,
                unix_ms: 123,
            }),
            Frame::Pong(Pong {
                token: 7,
                unix_ms: 456,
            }),
            Frame::SetLocale(SetLocale {
                locale: crate::Locale::Fr,
            }),
            Frame::BatteryStatus(BatteryStatus {
                percent: 42,
                charging: true,
            }),
            Frame::NavEnded,
        ];
        for f in frames {
            let bytes = encode_payload(&f).unwrap();
            let back = decode_payload(&bytes).unwrap();
            assert_eq!(f, back);
        }
    }

    #[test]
    fn json_projection_is_available_for_debugging() {
        let f = Frame::Hello(Hello {
            protocol_version: crate::PROTOCOL_VERSION,
            role: Role::Companion,
            app_version: "0.1.0".into(),
            device_name: "giorgio-pc".into(),
            identity_pub: "AAAA".into(),
            capabilities: vec!["locale".into()],
        });
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("Hello"));
        assert!(json.contains("giorgio-pc"));
    }
}
