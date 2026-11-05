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
//! [`encode_payload`] and [`decode_payload`] handle the payload alone. [`frame_bytes`] and
//! [`FrameDecoder`] add the length prefix for callers that own their byte stream and run no
//! async runtime, such as the phone apps. `omg-transport` has its own async codec for the
//! same layout.

use crate::{error::WireError, messages::Frame};

/// Hard cap on a single frame's payload. Large media/OTA transfers are chunked well
/// below this; the cap exists to bound memory on the Glass and reject garbage lengths.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;
/// Bytes in the length prefix.
const LEN_PREFIX: usize = 4;

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
    let (frame, rest) = postcard::take_from_bytes(bytes)?;
    // Postcard ignores what follows a value. Bytes that nothing authenticates have no place around
    // an authenticated frame; other frames may be followed by fields of a newer peer's.
    if matches!(frame, Frame::Sealed(_)) && !rest.is_empty() {
        return Err(WireError::TrailingBytes);
    }
    Ok(frame)
}

/// Encodes one frame with its length prefix.
pub fn frame_bytes(frame: &Frame) -> Result<Vec<u8>, WireError> {
    let payload = encode_payload(frame)?;
    let mut out = Vec::with_capacity(LEN_PREFIX + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Turns a byte stream into frames: push bytes as they arrive, take whole frames out.
///
/// An error is final, because the stream cannot be resynchronised. Close the link.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
    start: usize,
}

impl FrameDecoder {
    /// An empty decoder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends received bytes.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.start > 0 {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete frame, if one is buffered.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, WireError> {
        let pending = &self.buf[self.start..];
        let Some(prefix) = pending.first_chunk::<LEN_PREFIX>() else {
            return Ok(None);
        };
        let len = u32::from_be_bytes(*prefix) as usize;
        if len > MAX_FRAME_LEN {
            return Err(WireError::FrameTooLarge(len));
        }
        let Some(payload) = pending.get(LEN_PREFIX..LEN_PREFIX + len) else {
            return Ok(None);
        };
        let frame = decode_payload(payload)?;
        self.start += LEN_PREFIX + len;
        Ok(Some(frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::*;
    use crate::notes::MAX_NOTE_CHUNK_LEN;

    #[test]
    fn decoder_reassembles_frames_split_across_reads() {
        let a = Frame::Ping(Ping {
            token: 1,
            unix_ms: 2,
        });
        let b = Frame::NavEnded;
        let mut bytes = frame_bytes(&a).unwrap();
        bytes.extend(frame_bytes(&b).unwrap());
        let mut decoder = FrameDecoder::new();
        let mut got = Vec::new();
        for byte in bytes {
            decoder.push(&[byte]);
            while let Some(frame) = decoder.next_frame().unwrap() {
                got.push(frame);
            }
        }
        assert_eq!(got, vec![a, b]);
    }

    #[test]
    fn decoder_refuses_a_garbage_length_and_a_garbage_payload() {
        let mut decoder = FrameDecoder::new();
        decoder.push(&u32::MAX.to_be_bytes());
        assert!(matches!(
            decoder.next_frame(),
            Err(WireError::FrameTooLarge(_))
        ));
        let mut decoder = FrameDecoder::new();
        decoder.push(&[0, 0, 0, 1, 0xff]);
        assert!(decoder.next_frame().is_err());
    }

    #[test]
    fn a_sealed_frame_is_exact_and_costs_a_few_bytes() {
        let ciphertext = vec![7u8; MAX_NOTE_CHUNK_LEN as usize];
        let frame = Frame::Sealed(SealedFrame {
            ciphertext: ciphertext.clone(),
        });
        let mut bytes = encode_payload(&frame).unwrap();
        // One or two bytes of variant, and the length of the ciphertext as a varint.
        assert!(bytes.len() <= ciphertext.len() + 2 + 4);
        assert_eq!(decode_payload(&bytes).unwrap(), frame);
        bytes.push(0);
        assert!(matches!(
            decode_payload(&bytes),
            Err(WireError::TrailingBytes)
        ));
    }

    #[test]
    fn the_session_variants_are_the_last_ones() {
        // Positional on the wire: anything added before them breaks every peer.
        let sealed = encode_payload(&Frame::Sealed(SealedFrame {
            ciphertext: Vec::new(),
        }))
        .unwrap();
        let share = encode_payload(&Frame::SessionKeyShare(SessionKeyShare {
            ephemeral_pub: Vec::new(),
        }))
        .unwrap();
        let release = encode_payload(&Frame::NoteTtsRelease(crate::notes::NoteTtsRelease {
            request_id: 0,
        }))
        .unwrap();
        assert_eq!(share[0], release[0] + 1);
        assert_eq!(sealed[0], share[0] + 2);
    }

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
