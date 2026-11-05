//! Length-delimited framing of [`omg_protocol::Frame`] over a byte stream.
//!
//! Wire layout per frame: `u32` big-endian payload length, then the postcard payload
//! (see [`omg_protocol::wire`]). This is the exact format glassd must implement.
//!
//! Once a [`FrameCipher`] is installed the same codec carries every frame sealed, so whoever
//! holds the link or its split halves cannot send or accept a frame in clear by mistake.

use std::{
    collections::VecDeque,
    future::poll_fn,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Buf, BufMut, BytesMut};
use futures::{
    stream::{SplitSink, SplitStream},
    Sink, SinkExt, Stream as _, StreamExt,
};
use omg_protocol::{wire, Frame, MAX_FRAME_LEN};
use tokio_util::codec::{Decoder, Encoder, Framed};

use crate::{BoxStream, Stream, TransportError};

/// Number of bytes in the length prefix.
const LEN_PREFIX: usize = 4;

/// Protects frames on an established link. `omg-core` implements it, so that this crate stays
/// free of keys.
pub trait FrameCipher: Send {
    /// Turns an outgoing frame into what goes on the wire.
    fn seal(&mut self, frame: &Frame) -> Result<Frame, omg_protocol::WireError>;
    /// Turns what came off the wire into the frame it protects. An error ends the link.
    fn open(&mut self, frame: Frame) -> Result<Frame, omg_protocol::WireError>;
}

/// Encodes/decodes [`Frame`]s as `len:u32be || postcard(Frame)`, through a [`FrameCipher`] once
/// one is installed.
#[derive(Default)]
pub struct FrameCodec {
    cipher: Option<Box<dyn FrameCipher>>,
}

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
        match &mut self.cipher {
            Some(cipher) => Ok(Some(cipher.open(frame)?)),
            None => Ok(Some(frame)),
        }
    }
}

impl Encoder<Frame> for FrameCodec {
    type Error = TransportError;

    fn encode(&mut self, item: Frame, dst: &mut BytesMut) -> Result<(), TransportError> {
        let payload = match &mut self.cipher {
            Some(cipher) => wire::encode_payload(&cipher.seal(&item)?)?,
            None => wire::encode_payload(&item)?,
        };
        dst.reserve(LEN_PREFIX + payload.len());
        dst.put_u32(payload.len() as u32);
        dst.put_slice(&payload);
        Ok(())
    }
}

/// A framed, bidirectional [`Frame`] channel over a byte stream.
pub struct Link<S> {
    inner: Framed<S, FrameCodec>,
    /// Frames that arrived during a [`Link::send_reading`], not yet asked for. Empty once the
    /// handshake is over.
    pending: VecDeque<Frame>,
}

impl<S: Stream> Link<S> {
    /// Wrap a byte stream in the frame codec.
    pub fn new(stream: S) -> Self {
        Link {
            inner: Framed::new(stream, FrameCodec::default()),
            pending: VecDeque::new(),
        }
    }

    /// Protects every frame from here on. The frame last sent and the frame last received were
    /// the final ones in clear: frames already buffered are decoded only when asked for.
    pub fn set_cipher(&mut self, cipher: Box<dyn FrameCipher>) {
        self.inner.codec_mut().cipher = Some(cipher);
    }

    /// Whether a [`FrameCipher`] is protecting the link.
    pub fn is_encrypted(&self) -> bool {
        self.inner.codec().cipher.is_some()
    }

    /// Send one frame.
    pub async fn send(&mut self, frame: Frame) -> Result<(), TransportError> {
        self.inner.send(frame).await
    }

    /// Send one frame, reading while the write is stuck. Both ends of the handshake write before
    /// they read, so on a link with small buffers two plain sends would wait for each other for
    /// ever. What arrives meanwhile is kept for [`Link::recv`].
    pub async fn send_reading(&mut self, frame: Frame) -> Result<(), TransportError> {
        let inner = &mut self.inner;
        let pending = &mut self.pending;
        let mut closed = false;
        let mut pump = |inner: &mut Framed<S, FrameCodec>,
                        cx: &mut Context<'_>|
         -> Result<(), TransportError> {
            while !closed {
                match Pin::new(&mut *inner).poll_next(cx) {
                    Poll::Ready(Some(frame)) => pending.push_back(frame?),
                    Poll::Ready(None) => closed = true,
                    Poll::Pending => break,
                }
            }
            Ok(())
        };
        poll_fn(|cx| {
            let ready = Pin::new(&mut *inner).poll_ready(cx);
            if ready.is_pending() {
                pump(inner, cx)?;
            }
            ready
        })
        .await?;
        Pin::new(&mut *inner).start_send(frame)?;
        poll_fn(|cx| {
            let flushed = Pin::new(&mut *inner).poll_flush(cx);
            if flushed.is_pending() {
                pump(inner, cx)?;
            }
            flushed
        })
        .await
    }

    /// Receive the next frame, or `None` at end of stream.
    pub async fn recv(&mut self) -> Result<Option<Frame>, TransportError> {
        if let Some(frame) = self.pending.pop_front() {
            // Decoded before a cipher was installed, if one was since.
            return match &mut self.inner.codec_mut().cipher {
                Some(cipher) => Ok(Some(cipher.open(frame)?)),
                None => Ok(Some(frame)),
            };
        }
        match self.inner.next().await {
            Some(r) => r.map(Some),
            None => Ok(None),
        }
    }

    /// Split into independent send and receive halves, so a peer can keep reading while
    /// it writes. Long transfers need this: if both sides only read between their
    /// writes, full socket buffers can deadlock them.
    #[allow(clippy::type_complexity)]
    pub fn split(
        self,
    ) -> (
        SplitSink<Framed<S, FrameCodec>, Frame>,
        SplitStream<Framed<S, FrameCodec>>,
    ) {
        // The peer sends nothing more until it has read all that send_reading wrote, and the
        // handshake reads to its last frame, so nothing is left here to lose.
        debug_assert!(self.pending.is_empty());
        self.inner.split()
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
    async fn two_ends_writing_more_than_the_buffer_at_once_both_finish() {
        let (a, b) = tokio::io::duplex(8);
        let (mut la, mut lb) = (Link::new(a), Link::new(b));
        let big = Frame::Error(omg_protocol::ErrorMsg {
            code: omg_protocol::ErrorCode::Internal,
            message: "x".repeat(4096),
        });
        // As in the handshake: each writes, and then reads what the other wrote.
        let (ra, rb) = tokio::join!(
            async {
                la.send_reading(big.clone()).await.unwrap();
                la.recv().await
            },
            async {
                lb.send_reading(big.clone()).await.unwrap();
                lb.recv().await
            }
        );
        assert_eq!(ra.unwrap().unwrap(), big);
        assert_eq!(rb.unwrap().unwrap(), big);
    }

    #[tokio::test]
    async fn clean_eof_returns_none() {
        let (a, b) = tokio::io::duplex(64);
        let mut lb = Link::new(b);
        drop(a);
        assert!(lb.recv().await.unwrap().is_none());
    }
}
