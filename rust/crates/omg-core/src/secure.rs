//! Encrypts the link: the key exchange after mutual auth, and the sealing of every later frame.
//!
//! This is the one implementation. `session.rs`, `companion_link.rs` and glassd's link each drive
//! it from their own handshake, and nothing else touches a key. Nothing here does I/O or knows the
//! transport; docs/protocol/link-encryption.md has the protocol and the reasons for it.
//!
//! ```text
//! each side, after AuthResult:   SessionKeyShare { e }
//! on the peer's share:           SessionKeyProof { sign(identity, context || role || T) }
//! on the peer's proof, verified: keys = HKDF(salt = T, ikm = X25519(e, e_peer))
//! from then on:                  Sealed { ChaCha20-Poly1305(key, counter, postcard(Frame)) }
//! ```
//!
//! `T` hashes both `Hello`s, both challenge nonces, both identities and both shares, so the
//! signatures commit each side to all of what the other saw. The nonce is a counter neither side
//! sends: a dropped, repeated or reordered frame fails its tag, and any failure ends the link.

use std::sync::atomic::{AtomicU64, Ordering};

use chacha20poly1305::{
    aead::{AeadInPlace, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use hkdf::Hkdf;
use omg_protocol::{
    decode_payload, encode_payload, wire, ErrorCode, ErrorMsg, Frame, Hello, Role, SealedFrame,
    SessionKeyProof, SessionKeyShare, WireError, MAX_FRAME_LEN,
};
use sha2::{Digest, Sha256};
use thiserror::Error;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, Zeroizing};

use crate::identity::Identity;

/// Bytes a [`Frame`] gains from being sealed, at most: tag, variant, and ciphertext length. A
/// little more than needed, so that the limit is a round number well clear of the arithmetic.
pub const SEAL_OVERHEAD: usize = 32;
/// The largest `postcard(Frame)` that fits in a frame once sealed.
pub const MAX_SEALED_PLAINTEXT: usize = MAX_FRAME_LEN - SEAL_OVERHEAD;

const NONCE_LEN: usize = 32;
const SHARE_LEN: usize = 32;
const TAG_LEN: usize = 16;

/// Starts the transcript hash.
const TRANSCRIPT_CONTEXT: &[u8] = b"openmyglass-session-transcript-v1";
/// Signed before the transcript. It differs from `AUTH_CONTEXT` within its first bytes, so a
/// signature from the challenge exchange, where the peer picks what follows the context, can never
/// serve as a proof.
const PROOF_CONTEXT: &[u8] = b"omg-session-proof-v1";
const KEY_INFO_COMPANION_TO_GLASS: &[u8] = b"openmyglass-session-key-v1 companion to glass";
const KEY_INFO_GLASS_TO_COMPANION: &[u8] = b"openmyglass-session-key-v1 glass to companion";
const HEDGE_CONTEXT: &[u8] = b"openmyglass-hedged-random-v1";

/// Why the encrypted session cannot go on. Every one of these ends the link.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SecureError {
    /// A handshake value has the wrong size, or a side claims the wrong role.
    #[error("bad handshake value: {0}")]
    BadHandshake(&'static str),
    /// The peer's share is a low-order point or a copy of ours, so the exchange gives no secret.
    #[error("the key share gives no shared secret")]
    WeakShare,
    /// The signature over the handshake did not verify against the bonded key.
    #[error("the key exchange proof did not verify")]
    BadProof,
    /// A sealed frame did not decrypt: tampered, replayed, reordered, or a frame is missing.
    #[error("a frame failed authentication")]
    Tampered,
    /// A frame that is not sealed arrived on the encrypted link.
    #[error("{0} arrived unencrypted on the encrypted link")]
    Unsealed(&'static str),
    /// An authenticated frame could not be decoded, or a sealed frame held a sealed frame.
    #[error("bad frame inside a sealed frame: {0}")]
    BadInner(String),
    /// A frame failed earlier, so this link is over.
    #[error("the encrypted link has failed")]
    Dead,
    /// This frame cannot be sealed. No nonce was used, and the link goes on.
    #[error("frame of {0} bytes is over the sealed limit")]
    TooLarge(usize),
    /// A frame could not be encoded.
    #[error("cannot encode a frame: {0}")]
    Encode(String),
    /// An application frame was about to leave before the keys exist.
    #[error("{0} refused: the link is not encrypted yet")]
    NotYet(&'static str),
}

impl From<SecureError> for WireError {
    fn from(e: SecureError) -> Self {
        WireError::Rejected(e.to_string())
    }
}

/// The frame a side sends and closes with when the peer does not advertise
/// [`omg_protocol::capability::SESSION_CRYPTO`].
pub fn refuse_unencrypted_peer() -> Frame {
    Frame::Error(ErrorMsg {
        code: ErrorCode::Unsupported,
        message: "this peer requires an encrypted link; update the other side".into(),
    })
}

/// Whether a [`Hello`] advertises the encrypted session.
pub fn peer_supports_crypto(hello: &Hello) -> bool {
    hello
        .capabilities
        .iter()
        .any(|c| c == omg_protocol::capability::SESSION_CRYPTO)
}

/// Adds [`omg_protocol::capability::SESSION_CRYPTO`] to a capability list, if a caller left it out:
/// no configuration may turn encryption off.
pub fn with_crypto_capability(mut capabilities: Vec<String>) -> Vec<String> {
    let cap = omg_protocol::capability::SESSION_CRYPTO;
    if !capabilities.iter().any(|c| c == cap) {
        capabilities.push(cap.to_string());
    }
    capabilities
}

/// What may cross the link while there are no keys: the handshake, and the words that end it.
pub fn allowed_in_clear(frame: &Frame) -> bool {
    matches!(
        frame,
        Frame::Hello(_)
            | Frame::PairRequest(_)
            | Frame::PairAccept(_)
            | Frame::PairResult(_)
            | Frame::AuthChallenge(_)
            | Frame::AuthResponse(_)
            | Frame::AuthResult(_)
            | Frame::SessionKeyShare(_)
            | Frame::SessionKeyProof(_)
            | Frame::Error(_)
            | Frame::Bye(_)
    )
}

/// 32 bytes from the OS and from the identity's secret, for a value that must never repeat.
///
/// On a kernel without `getrandom(2)`, `/dev/urandom` can be thin when a Glass boots without its
/// seed file (`glassd/src/companion/entropy.rs`), while the identity was made behind a stricter
/// gate. So a secret or nonce made here is as strong as the better of the two, as RFC 8937 does it:
/// the stored secret stands in for a weak pool, and a healthy pool keeps the value independent of
/// the stored secret. The clock and a counter keep it fresh if the pool repeats across boots.
pub(crate) fn hedged_random(identity: &Identity, label: &[u8], context: &[u8]) -> [u8; 32] {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut ikm = Zeroizing::new([0u8; 64]);
    let (os, seed) = ikm.split_at_mut(32);
    getrandom::getrandom(os).expect("OS RNG available");
    seed.copy_from_slice(identity.secret_seed().as_slice());
    let mut info = Sha256::new();
    absorb(&mut info, label);
    absorb(&mut info, context);
    absorb(&mut info, &crate::now_ms().to_be_bytes());
    absorb(
        &mut info,
        &COUNTER.fetch_add(1, Ordering::Relaxed).to_be_bytes(),
    );
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(Some(HEDGE_CONTEXT), ikm.as_slice())
        .expand(&info.finalize(), &mut out)
        .expect("32 bytes is a valid HKDF length");
    out
}

/// A length-prefixed field, so that no two sets of fields hash alike.
fn absorb(h: &mut Sha256, field: &[u8]) {
    h.update((field.len() as u64).to_be_bytes());
    h.update(field);
}

/// Everything in a `Hello` that either side acts on. Spelled out field by field rather than
/// re-encoded, so a future peer's trailing bytes cannot change what is hashed.
fn absorb_hello(h: &mut Sha256, hello: &Hello) {
    h.update(hello.protocol_version.to_be_bytes());
    h.update([role_byte(hello.role)]);
    absorb(h, hello.app_version.as_bytes());
    absorb(h, hello.device_name.as_bytes());
    absorb(h, hello.identity_pub.as_bytes());
    h.update((hello.capabilities.len() as u64).to_be_bytes());
    for c in &hello.capabilities {
        absorb(h, c.as_bytes());
    }
}

fn role_byte(role: Role) -> u8 {
    match role {
        Role::Companion => 0,
        Role::Glass => 1,
    }
}

/// What a side knows when the challenge exchange is over.
#[derive(Debug, Clone, Copy)]
pub struct HandshakeFacts<'a> {
    /// The `Hello` we sent.
    pub our_hello: &'a Hello,
    /// The `Hello` we received.
    pub peer_hello: &'a Hello,
    /// The nonce we challenged the peer with.
    pub our_nonce: &'a [u8],
    /// The nonce the peer challenged us with.
    pub peer_nonce: &'a [u8],
    /// The bonded peer's public key, base64.
    pub bonded_peer_pub: &'a str,
}

/// One side of the key exchange. See the module docs.
pub struct KeyExchange {
    role: Role,
    identity: Identity,
    peer_key: VerifyingKey,
    /// Both `Hello`s, both nonces, both identities.
    base: Sha256,
    secret: StaticSecret,
    our_share: [u8; SHARE_LEN],
    /// Set once the peer's share arrived.
    keys: Option<PendingKeys>,
}

impl std::fmt::Debug for KeyExchange {
    /// Nothing of the keys.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyExchange")
    }
}

struct PendingKeys {
    transcript: [u8; 32],
    companion_to_glass: Zeroizing<[u8; 32]>,
    glass_to_companion: Zeroizing<[u8; 32]>,
}

impl KeyExchange {
    /// Begins the exchange. Returns the [`SessionKeyShare`] to send.
    ///
    /// Call only after the peer's `AuthResult`, and so after the pool gate on a Glass: the
    /// ephemeral key comes from [`hedged_random`].
    pub fn start(
        identity: &Identity,
        facts: HandshakeFacts<'_>,
    ) -> Result<(KeyExchange, Frame), SecureError> {
        let role = facts.our_hello.role;
        if facts.peer_hello.role == role {
            return Err(SecureError::BadHandshake("both sides claim the same role"));
        }
        if facts.our_nonce.len() != NONCE_LEN || facts.peer_nonce.len() != NONCE_LEN {
            return Err(SecureError::BadHandshake("challenge nonce size"));
        }
        let peer_key = decode_key(facts.bonded_peer_pub)
            .ok_or(SecureError::BadHandshake("bonded peer key"))?;
        let our_key = decode_key(&identity.public_b64())
            .ok_or(SecureError::BadHandshake("our identity key"))?;
        // Always the companion's first, so that both sides hash the same thing.
        let (hello_c, hello_g, nonce_c, nonce_g, key_c, key_g) = match role {
            Role::Companion => (
                facts.our_hello,
                facts.peer_hello,
                facts.our_nonce,
                facts.peer_nonce,
                &our_key,
                &peer_key,
            ),
            Role::Glass => (
                facts.peer_hello,
                facts.our_hello,
                facts.peer_nonce,
                facts.our_nonce,
                &peer_key,
                &our_key,
            ),
        };
        let mut base = Sha256::new();
        absorb(&mut base, TRANSCRIPT_CONTEXT);
        absorb_hello(&mut base, hello_c);
        absorb_hello(&mut base, hello_g);
        absorb(&mut base, nonce_c);
        absorb(&mut base, nonce_g);
        absorb(&mut base, key_c.as_bytes());
        absorb(&mut base, key_g.as_bytes());

        let mut random = hedged_random(identity, b"ephemeral", &base.clone().finalize());
        let secret = StaticSecret::from(random);
        random.zeroize();
        let our_share = PublicKey::from(&secret).to_bytes();
        Ok((
            KeyExchange {
                role,
                identity: identity.clone(),
                peer_key,
                base,
                secret,
                our_share,
                keys: None,
            },
            Frame::SessionKeyShare(SessionKeyShare {
                ephemeral_pub: our_share.to_vec(),
            }),
        ))
    }

    /// The peer's share arrived. Returns the [`SessionKeyProof`] to send.
    pub fn on_share(&mut self, share: &SessionKeyShare) -> Result<Frame, SecureError> {
        if self.keys.is_some() {
            return Err(SecureError::BadHandshake("a second key share"));
        }
        let peer_share: [u8; SHARE_LEN] = share
            .ephemeral_pub
            .as_slice()
            .try_into()
            .map_err(|_| SecureError::BadHandshake("key share size"))?;
        if peer_share == self.our_share {
            return Err(SecureError::WeakShare);
        }
        let shared = self.secret.diffie_hellman(&PublicKey::from(peer_share));
        if !shared.was_contributory() {
            return Err(SecureError::WeakShare);
        }
        let (share_c, share_g) = match self.role {
            Role::Companion => (&self.our_share, &peer_share),
            Role::Glass => (&peer_share, &self.our_share),
        };
        let mut h = self.base.clone();
        absorb(&mut h, share_c);
        absorb(&mut h, share_g);
        let transcript: [u8; 32] = h.finalize().into();

        let hkdf = Hkdf::<Sha256>::new(Some(&transcript), shared.as_bytes());
        let mut companion_to_glass = Zeroizing::new([0u8; 32]);
        let mut glass_to_companion = Zeroizing::new([0u8; 32]);
        hkdf.expand(KEY_INFO_COMPANION_TO_GLASS, companion_to_glass.as_mut())
            .and_then(|_| hkdf.expand(KEY_INFO_GLASS_TO_COMPANION, glass_to_companion.as_mut()))
            .expect("32 bytes is a valid HKDF length");

        let signature = self
            .identity
            .sign_raw(&proof_message(self.role, &transcript));
        self.keys = Some(PendingKeys {
            transcript,
            companion_to_glass,
            glass_to_companion,
        });
        Ok(Frame::SessionKeyProof(SessionKeyProof {
            signature: signature.to_vec(),
        }))
    }

    /// The peer's proof arrived, after its share. On success the link is encrypted from here on.
    pub fn on_proof(self, proof: &SessionKeyProof) -> Result<Channel, SecureError> {
        let keys = self
            .keys
            .as_ref()
            .ok_or(SecureError::BadHandshake("a proof before the key share"))?;
        let peer_role = match self.role {
            Role::Companion => Role::Glass,
            Role::Glass => Role::Companion,
        };
        let signature =
            Signature::from_slice(&proof.signature).map_err(|_| SecureError::BadProof)?;
        self.peer_key
            .verify(&proof_message(peer_role, &keys.transcript), &signature)
            .map_err(|_| SecureError::BadProof)?;
        let (tx, rx) = match self.role {
            Role::Companion => (&keys.companion_to_glass, &keys.glass_to_companion),
            Role::Glass => (&keys.glass_to_companion, &keys.companion_to_glass),
        };
        Ok(Channel {
            tx: Direction::new(tx.as_slice()),
            rx: Direction::new(rx.as_slice()),
            dead: false,
        })
    }
}

fn decode_key(b64: &str) -> Option<VerifyingKey> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    let bytes: [u8; 32] = B64.decode(b64.trim()).ok()?.try_into().ok()?;
    VerifyingKey::from_bytes(&bytes).ok()
}

/// What a side signs: which role it speaks as, so that no proof can be sent back to its author.
fn proof_message(signer: Role, transcript: &[u8; 32]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(PROOF_CONTEXT.len() + 1 + transcript.len());
    msg.extend_from_slice(PROOF_CONTEXT);
    msg.push(role_byte(signer));
    msg.extend_from_slice(transcript);
    msg
}

struct Direction {
    aead: ChaCha20Poly1305,
    /// The nonce of the next frame.
    counter: u64,
}

impl Direction {
    fn new(key: &[u8]) -> Direction {
        Direction {
            aead: ChaCha20Poly1305::new(Key::from_slice(key)),
            counter: 0,
        }
    }

    fn nonce(&self) -> Nonce {
        let mut nonce = [0u8; 12];
        nonce[4..].copy_from_slice(&self.counter.to_be_bytes());
        nonce.into()
    }
}

/// The encrypted link: seals frames going out and opens frames coming in, each direction with its
/// own key and counter. Keys are wiped on drop.
pub struct Channel {
    tx: Direction,
    rx: Direction,
    dead: bool,
}

impl Channel {
    /// Encrypts one frame. Frames must then reach the peer in the order they were sealed.
    pub fn seal(&mut self, frame: &Frame) -> Result<Frame, SecureError> {
        if self.dead {
            return Err(SecureError::Dead);
        }
        if matches!(frame, Frame::Sealed(_)) {
            return Err(SecureError::BadInner("sealing a sealed frame".into()));
        }
        let mut buf = encode_payload(frame).map_err(|e| SecureError::Encode(e.to_string()))?;
        // Before the nonce is used: a frame refused here must not leave a gap in the counter.
        if buf.len() > MAX_SEALED_PLAINTEXT {
            return Err(SecureError::TooLarge(buf.len()));
        }
        if self.tx.counter == u64::MAX {
            self.dead = true;
            return Err(SecureError::Dead);
        }
        buf.reserve_exact(TAG_LEN);
        let nonce = self.tx.nonce();
        self.tx
            .aead
            .encrypt_in_place(&nonce, b"", &mut buf)
            .map_err(|_| SecureError::TooLarge(buf.len()))?;
        self.tx.counter += 1;
        Ok(Frame::Sealed(SealedFrame { ciphertext: buf }))
    }

    /// Decrypts one frame from the link. Anything but the next sealed frame of this session is an
    /// error, and after one error every later call fails too: the link must be closed.
    pub fn open(&mut self, frame: Frame) -> Result<Frame, SecureError> {
        if self.dead {
            return Err(SecureError::Dead);
        }
        let result = self.open_inner(frame);
        self.dead = result.is_err();
        result
    }

    fn open_inner(&mut self, frame: Frame) -> Result<Frame, SecureError> {
        let Frame::Sealed(SealedFrame { mut ciphertext }) = frame else {
            return Err(SecureError::Unsealed(frame.wire_name()));
        };
        let nonce = self.rx.nonce();
        self.rx
            .aead
            .decrypt_in_place(&nonce, b"", &mut ciphertext)
            .map_err(|_| SecureError::Tampered)?;
        self.rx.counter += 1;
        let inner =
            decode_payload(&ciphertext).map_err(|e| SecureError::BadInner(e.to_string()))?;
        if matches!(inner, Frame::Sealed(_)) {
            return Err(SecureError::BadInner(
                "a sealed frame inside another".into(),
            ));
        }
        Ok(inner)
    }

    /// Encodes one frame for the socket: sealed, with its length prefix.
    pub fn seal_bytes(&mut self, frame: &Frame) -> Result<Vec<u8>, SecureError> {
        let sealed = self.seal(frame)?;
        wire::frame_bytes(&sealed).map_err(|e| SecureError::Encode(e.to_string()))
    }
}

/// Encodes a frame for a socket owned by a frontend: sealed once `channel` exists, and refused
/// if it is an application frame and there are no keys yet, so that a mistake in a frontend can
/// stop a frame but never send it in clear.
pub fn encode_frame(channel: Option<&mut Channel>, frame: &Frame) -> Result<Vec<u8>, SecureError> {
    match channel {
        Some(channel) => channel.seal_bytes(frame),
        None if allowed_in_clear(frame) => {
            wire::frame_bytes(frame).map_err(|e| SecureError::Encode(e.to_string()))
        }
        None => Err(SecureError::NotYet(frame.wire_name())),
    }
}

#[cfg(feature = "async-link")]
impl omg_transport::FrameCipher for Channel {
    fn seal(&mut self, frame: &Frame) -> Result<Frame, WireError> {
        Channel::seal(self, frame).map_err(Into::into)
    }

    fn open(&mut self, frame: Frame) -> Result<Frame, WireError> {
        Channel::open(self, frame).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omg_protocol::{capability, NoteChunk, NoteId, Ping, MAX_NOTE_CHUNK_LEN};

    fn hello(identity: &Identity, role: Role) -> Hello {
        Hello {
            protocol_version: omg_protocol::PROTOCOL_VERSION,
            role,
            app_version: "0".into(),
            device_name: "device".into(),
            identity_pub: identity.public_b64(),
            capabilities: vec![capability::SESSION_CRYPTO.to_string()],
        }
    }

    struct Pair {
        companion: Channel,
        glass: Channel,
    }

    /// Runs the exchange with the shares and proofs each side really makes.
    fn pair() -> Pair {
        let (c_id, g_id) = (Identity::generate(), Identity::generate());
        let (c_hello, g_hello) = (hello(&c_id, Role::Companion), hello(&g_id, Role::Glass));
        let (n_c, n_g) = ([1u8; 32], [2u8; 32]);
        let (mut c, c_share) = KeyExchange::start(
            &c_id,
            HandshakeFacts {
                our_hello: &c_hello,
                peer_hello: &g_hello,
                our_nonce: &n_c,
                peer_nonce: &n_g,
                bonded_peer_pub: &g_id.public_b64(),
            },
        )
        .unwrap();
        let (mut g, g_share) = KeyExchange::start(
            &g_id,
            HandshakeFacts {
                our_hello: &g_hello,
                peer_hello: &c_hello,
                our_nonce: &n_g,
                peer_nonce: &n_c,
                bonded_peer_pub: &c_id.public_b64(),
            },
        )
        .unwrap();
        let (Frame::SessionKeyShare(c_share), Frame::SessionKeyShare(g_share)) = (c_share, g_share)
        else {
            panic!("not shares");
        };
        let c_proof = c.on_share(&g_share).unwrap();
        let g_proof = g.on_share(&c_share).unwrap();
        let (Frame::SessionKeyProof(c_proof), Frame::SessionKeyProof(g_proof)) = (c_proof, g_proof)
        else {
            panic!("not proofs");
        };
        Pair {
            companion: c.on_proof(&g_proof).unwrap(),
            glass: g.on_proof(&c_proof).unwrap(),
        }
    }

    fn ping(token: u64) -> Frame {
        Frame::Ping(Ping { token, unix_ms: 0 })
    }

    #[test]
    fn both_directions_carry_frames() {
        let mut p = pair();
        for token in 0..3 {
            let sealed = p.companion.seal(&ping(token)).unwrap();
            assert_eq!(p.glass.open(sealed).unwrap(), ping(token));
            let sealed = p.glass.seal(&ping(token)).unwrap();
            assert_eq!(p.companion.open(sealed).unwrap(), ping(token));
        }
    }

    #[test]
    fn a_frame_cannot_come_back_to_its_sender() {
        let mut p = pair();
        let sealed = p.companion.seal(&ping(1)).unwrap();
        assert_eq!(p.companion.open(sealed), Err(SecureError::Tampered));
    }

    #[test]
    fn a_replayed_frame_ends_the_link() {
        let mut p = pair();
        let sealed = p.companion.seal(&ping(1)).unwrap();
        p.glass.open(sealed.clone()).unwrap();
        assert_eq!(p.glass.open(sealed), Err(SecureError::Tampered));
    }

    #[test]
    fn a_reordered_frame_ends_the_link() {
        let mut p = pair();
        let first = p.companion.seal(&ping(1)).unwrap();
        let second = p.companion.seal(&ping(2)).unwrap();
        assert_eq!(p.glass.open(second), Err(SecureError::Tampered));
        assert_eq!(p.glass.open(first), Err(SecureError::Dead));
    }

    #[test]
    fn a_lost_frame_ends_the_link() {
        let mut p = pair();
        let _lost = p.companion.seal(&ping(1)).unwrap();
        let second = p.companion.seal(&ping(2)).unwrap();
        assert_eq!(p.glass.open(second), Err(SecureError::Tampered));
    }

    #[test]
    fn a_changed_bit_ends_the_link_and_the_next_good_frame_stays_refused() {
        let mut p = pair();
        let Frame::Sealed(mut sealed) = p.companion.seal(&ping(1)).unwrap() else {
            panic!();
        };
        let good = Frame::Sealed(sealed.clone());
        sealed.ciphertext[0] ^= 1;
        assert_eq!(
            p.glass.open(Frame::Sealed(sealed)),
            Err(SecureError::Tampered)
        );
        assert_eq!(p.glass.open(good), Err(SecureError::Dead));
    }

    #[test]
    fn a_frame_in_clear_ends_the_link() {
        let mut p = pair();
        assert_eq!(
            p.glass.open(Frame::WifiTearDown),
            Err(SecureError::Unsealed("WifiTearDown"))
        );
        let sealed = p.companion.seal(&ping(1)).unwrap();
        assert_eq!(p.glass.open(sealed), Err(SecureError::Dead));
    }

    #[test]
    fn another_session_has_other_keys() {
        let mut one = pair();
        let mut other = pair();
        let sealed = one.companion.seal(&ping(1)).unwrap();
        assert_eq!(other.glass.open(sealed), Err(SecureError::Tampered));
    }

    #[test]
    fn the_largest_note_chunk_fits_and_stays_far_under_the_frame_limit() {
        let mut p = pair();
        let chunk = Frame::NoteChunk(NoteChunk {
            note_id: NoteId([3; 16]),
            attachment_id: u16::MAX,
            offset: u64::MAX,
            data: vec![0xff; MAX_NOTE_CHUNK_LEN as usize],
            sha256: [4; 32],
        });
        let sealed = p.companion.seal_bytes(&chunk).unwrap();
        // A sealed frame costs little over a plain one.
        let plain = wire::frame_bytes(&chunk).unwrap();
        assert!(sealed.len() <= plain.len() + SEAL_OVERHEAD);
        assert!(sealed.len() < MAX_FRAME_LEN / 100);
        let mut decoder = omg_protocol::FrameDecoder::new();
        decoder.push(&sealed);
        let outer = decoder.next_frame().unwrap().unwrap();
        assert_eq!(p.glass.open(outer).unwrap(), chunk);
    }

    #[test]
    fn the_seal_overhead_is_honest_at_the_limit() {
        let mut p = pair();
        // Find the Frame::Error whose encoding is exactly at the limit.
        let frame_of = |len: usize| {
            Frame::Error(ErrorMsg {
                code: ErrorCode::Internal,
                message: "x".repeat(len),
            })
        };
        let overhead = encode_payload(&frame_of(MAX_SEALED_PLAINTEXT))
            .unwrap()
            .len()
            - MAX_SEALED_PLAINTEXT;
        let at_limit = frame_of(MAX_SEALED_PLAINTEXT - overhead);
        assert_eq!(
            encode_payload(&at_limit).unwrap().len(),
            MAX_SEALED_PLAINTEXT
        );
        let sealed = p.companion.seal_bytes(&at_limit).unwrap();
        assert!(sealed.len() - 4 <= MAX_FRAME_LEN);
        let mut decoder = omg_protocol::FrameDecoder::new();
        decoder.push(&sealed);
        let outer = decoder.next_frame().unwrap().unwrap();
        assert_eq!(p.glass.open(outer).unwrap(), at_limit);
    }

    #[test]
    fn a_frame_too_large_is_refused_without_a_gap_in_the_counter() {
        let mut p = pair();
        let huge = Frame::Error(ErrorMsg {
            code: ErrorCode::Internal,
            message: "x".repeat(MAX_SEALED_PLAINTEXT + 1),
        });
        assert!(matches!(
            p.companion.seal(&huge),
            Err(SecureError::TooLarge(_))
        ));
        let sealed = p.companion.seal(&ping(1)).unwrap();
        assert_eq!(p.glass.open(sealed).unwrap(), ping(1));
    }

    #[test]
    fn only_the_handshake_may_leave_in_clear() {
        assert!(matches!(
            encode_frame(None, &Frame::WifiTearDown),
            Err(SecureError::NotYet("WifiTearDown"))
        ));
        assert!(encode_frame(
            None,
            &Frame::Bye(omg_protocol::Bye {
                reason: String::new()
            })
        )
        .is_ok());
    }

    #[test]
    fn hedged_values_differ_even_with_the_same_inputs() {
        let id = Identity::generate();
        assert_ne!(
            hedged_random(&id, b"label", b"context"),
            hedged_random(&id, b"label", b"context")
        );
    }

    #[test]
    fn a_low_order_share_is_refused() {
        let (c_id, g_id) = (Identity::generate(), Identity::generate());
        let (c_hello, g_hello) = (hello(&c_id, Role::Companion), hello(&g_id, Role::Glass));
        let (mut c, _) = KeyExchange::start(
            &c_id,
            HandshakeFacts {
                our_hello: &c_hello,
                peer_hello: &g_hello,
                our_nonce: &[1; 32],
                peer_nonce: &[2; 32],
                bonded_peer_pub: &g_id.public_b64(),
            },
        )
        .unwrap();
        assert_eq!(
            c.on_share(&SessionKeyShare {
                ephemeral_pub: vec![0; 32]
            }),
            Err(SecureError::WeakShare)
        );
    }

    #[test]
    fn a_challenge_signature_is_not_a_proof() {
        // The challenge exchange signs whatever the peer asks for. A relay asks for the bytes of
        // a proof, with every prefix it could try: the contexts must keep the two apart.
        assert!(!PROOF_CONTEXT.starts_with(crate::identity::AUTH_CONTEXT));
        assert!(!crate::identity::AUTH_CONTEXT.starts_with(PROOF_CONTEXT));
    }
}
