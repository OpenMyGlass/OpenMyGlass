//! Length-delimited framing of [`omg_protocol::Frame`] over a byte stream.
//!
//! Wire layout per frame: `u32` big-endian payload length, then the postcard payload
//! (see [`omg_protocol::wire`]). This is the exact format glassd must implement.

use bytes::{Buf, BufMut, BytesMut};
use futures::{SinkExt, StreamExt};
use omg_protocol::{wire, Frame, MAX_FRAME_LEN};
use tokio_util::codec::{Decoder, Encoder, Framed};

use crate::{BoxStream, Stream, TransportError};

/// Number of bytes in the length prefix.
const LEN_PREFIX: usize = 4;

/// Encodes/decodes [`Frame`]s as `len:u32be || postcard(Frame)`.
#[derive(Debug, Default, Clone)]
pub struct FrameCodec;

impl Decoder for FrameCodec {
    type Item = Frame;
    type Error = TransportError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Frame>, TransportError> {
        if src.len() < LEN_PREFIX {
            return Ok(None);
        }
        let len = u32::from_be_bytes([src[0], src[1], src[2], src[3]]) as usize;
        if len > MAX_FRAME_LEN {
            return Err(TransportError::Wire(
                omg_protocol::WireError::FrameTooLarge(len),
            ));
        }
        if src.len() < LEN_PREFIX + len {
            // Reserve so the next read has room; avoids repeated small growths.
            src.reserve(LEN_PREFIX + len - src.len());
            return Ok(None);
        }
        src.advance(LEN_PREFIX);
        let payload = src.split_to(len);
        let frame = wire::decode_payload(&payload)?;
        Ok(Some(frame))
    }
}

impl Encoder<Frame> for FrameCodec {
    type Error = TransportError;

    fn encode(&mut self, item: Frame, dst: &mut BytesMut) -> Result<(), TransportError> {
        let payload = wire::encode_payload(&item)?;
        dst.reserve(LEN_PREFIX + payload.len());
        dst.put_u32(payload.len() as u32);
        dst.put_slice(&payload);
        Ok(())
    }
}

/// A framed, bidirectional [`Frame`] channel over a byte stream.
pub struct Link<S> {
    inner: Framed<S, FrameCodec>,
}

impl<S: Stream> Link<S> {
    /// Wrap a byte stream in the frame codec.
    pub fn new(stream: S) -> Self {
        Link {
            inner: Framed::new(stream, FrameCodec),
        }
    }

    /// Send one frame.
    pub async fn send(&mut self, frame: Frame) -> Result<(), TransportError> {
        self.inner.send(frame).await
    }

    /// Receive the next frame, or `None` at end of stream.
    pub async fn recv(&mut self) -> Result<Option<Frame>, TransportError> {
        match self.inner.next().await {
            Some(r) => r.map(Some),
            None => Ok(None),
        }
    }
}

impl Link<BoxStream> {
    /// Box the stream so links over different transports share one type.
    pub fn boxed<S: Stream + 'static>(stream: S) -> Link<BoxStream> {
        Link::new(Box::new(stream) as BoxStream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omg_protocol::{BatteryStatus, Ping};

    #[tokio::test]
    async fn frames_roundtrip_over_duplex() {
        let (a, b) = tokio::io::duplex(1024);
        let mut la = Link::new(a);
        let mut lb = Link::new(b);

        la.send(Frame::Ping(Ping {
            token: 99,
            unix_ms: 1,
        }))
        .await
        .unwrap();
        la.send(Frame::BatteryStatus(BatteryStatus {
            percent: 55,
            charging: false,
        }))
        .await
        .unwrap();

        assert_eq!(
            lb.recv().await.unwrap().unwrap(),
            Frame::Ping(Ping {
                token: 99,
                unix_ms: 1
            })
        );
        assert_eq!(
            lb.recv().await.unwrap().unwrap(),
            Frame::BatteryStatus(BatteryStatus {
                percent: 55,
                charging: false
            })
        );
    }

    #[tokio::test]
    async fn clean_eof_returns_none() {
        let (a, b) = tokio::io::duplex(64);
        let mut lb = Link::new(b);
        drop(a);
        assert!(lb.recv().await.unwrap().is_none());
    }
}
