//! The companion's side of the handshake as a pure state machine.
//!
//! A frontend that owns its socket feeds it decoded frames and carries out what comes back.
//! It does the same as `session.rs`, which runs both roles as an async flow for the CLI and the
//! simulated Glass; `tests/companion_link.rs` runs this one against that one over TCP, so the
//! two cannot drift apart unnoticed.
//!
//! Nothing here depends on the kind of link. Over Bluetooth or Wi-Fi alike, the peer is proved
//! by its bonded ed25519 key, and every frame after the handshake is encrypted
//! ([`crate::secure`]). So a frontend turns frames into bytes with [`CompanionLink::encode`],
//! never with `frame_bytes`, and carries out each call's outputs before making the next call.

use omg_protocol::{
    AuthChallenge, AuthResponse, AuthResult, ErrorCode, ErrorMsg, Frame, Hello, PairRequest,
    PairResult, Pong, Role, PROTOCOL_VERSION,
};

use crate::{
    identity::{decode_nonce, random_nonce_b64, verify_challenge, Identity},
    now_ms,
    secure::{
        encode_frame, peer_supports_crypto, refuse_unencrypted_peer, with_crypto_capability,
        Channel, HandshakeFacts, KeyExchange, SecureError,
    },
    store::PairingRecord,
};

/// How a session should treat an unbonded peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingMode {
    /// Only talk to the already-bonded peer; refuse to bond a new one.
    ReuseOnly,
    /// Permit a one-time first-pairing with a new peer (user initiated "pair" action).
    AllowNew,
}

/// What the caller must do, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkOut {
    /// Send this frame.
    Send(Frame),
    /// Store this bond before sending anything that follows.
    Persist(PairingRecord),
    /// The peer is authenticated and the link encrypted.
    Running {
        /// The Glass's name.
        peer_name: String,
        /// What it advertised.
        capabilities: Vec<String>,
    },
    /// A frame for the application, once running.
    App(Frame),
    /// The link is over. Close the socket.
    Closed(Result<(), LinkEnd>),
}

/// Why a link ended, in a form that can be compared and copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEnd {
    /// The Glass is bonded to another companion, or this side to another Glass.
    UnknownPeer,
    /// No bond, and a new one was not permitted here or there.
    NotPaired,
    /// A signature did not verify.
    AuthFailed,
    /// The peer does not encrypt the link. It is refused, and no bond was stored.
    PeerNotEncrypted,
    /// A frame failed authentication: someone tampered with the link, or it corrupted a frame.
    Tampered,
    /// Incompatible protocol versions.
    VersionMismatch {
        /// Ours.
        ours: u16,
        /// Theirs.
        theirs: u16,
    },
    /// Anything else, for logs.
    Protocol(String),
}

#[derive(Debug)]
enum Phase {
    Start,
    AwaitingHello,
    AwaitingPairAccept { nonce: String, peer: Hello },
    Authenticating(Auth),
    AwaitingKeyShare { exchange: KeyExchange, peer: Hello },
    AwaitingKeyProof { exchange: KeyExchange, peer: Hello },
    Running,
    Closed,
}

#[derive(Debug)]
struct Auth {
    peer: Hello,
    our_nonce: Vec<u8>,
    peer_nonce: Option<Vec<u8>>,
    verified: Option<bool>,
}

/// See the module docs.
pub struct CompanionLink {
    identity: Identity,
    bonded: Option<PairingRecord>,
    mode: PairingMode,
    device_name: String,
    app_version: String,
    capabilities: Vec<String>,
    our_hello: Option<Hello>,
    channel: Option<Channel>,
    phase: Phase,
}

impl CompanionLink {
    /// A link not yet started. `bonded` is the remembered Glass, if any.
    pub fn new(
        identity: Identity,
        bonded: Option<PairingRecord>,
        mode: PairingMode,
        device_name: impl Into<String>,
        app_version: impl Into<String>,
        capabilities: Vec<String>,
    ) -> Self {
        CompanionLink {
            identity,
            bonded,
            mode,
            device_name: device_name.into(),
            app_version: app_version.into(),
            capabilities: with_crypto_capability(capabilities),
            our_hello: None,
            channel: None,
            phase: Phase::Start,
        }
    }

    /// Whether the peer is authenticated and the link encrypted.
    pub fn is_running(&self) -> bool {
        matches!(self.phase, Phase::Running)
    }

    /// Whether the link is over.
    pub fn is_closed(&self) -> bool {
        matches!(self.phase, Phase::Closed)
    }

    /// Sends our `Hello`. Call once the socket is open.
    pub fn start(&mut self) -> Vec<LinkOut> {
        if !matches!(self.phase, Phase::Start) {
            return Vec::new();
        }
        self.phase = Phase::AwaitingHello;
        let hello = Hello {
            protocol_version: PROTOCOL_VERSION,
            role: Role::Companion,
            app_version: self.app_version.clone(),
            device_name: self.device_name.clone(),
            identity_pub: self.identity.public_b64(),
            capabilities: self.capabilities.clone(),
        };
        self.our_hello = Some(hello.clone());
        vec![LinkOut::Send(Frame::Hello(hello))]
    }

    /// Turns a frame into the bytes to write to the socket: sealed once the link is encrypted.
    /// Refuses an application frame before that. Frames must reach the socket in the order they
    /// went through here.
    pub fn encode(&mut self, frame: &Frame) -> Result<Vec<u8>, SecureError> {
        encode_frame(self.channel.as_mut(), frame)
    }

    /// A frame arrived.
    pub fn on_frame(&mut self, frame: Frame) -> Vec<LinkOut> {
        let mut out = Vec::new();
        match std::mem::replace(&mut self.phase, Phase::Closed) {
            Phase::Start | Phase::Closed => {}
            Phase::AwaitingHello => self.on_hello(frame, &mut out),
            Phase::AwaitingPairAccept { nonce, peer } => {
                self.on_pair_accept(frame, nonce, peer, &mut out)
            }
            Phase::Authenticating(auth) => self.on_auth(frame, auth, &mut out),
            Phase::AwaitingKeyShare { exchange, peer } => {
                self.on_key_share(frame, exchange, peer, &mut out)
            }
            Phase::AwaitingKeyProof { exchange, peer } => {
                self.on_key_proof(frame, exchange, peer, &mut out)
            }
            Phase::Running => self.on_running(frame, &mut out),
        }
        out
    }

    /// Says goodbye and ends the link.
    pub fn close(&mut self, reason: &str) -> Vec<LinkOut> {
        if matches!(self.phase, Phase::Closed) {
            return Vec::new();
        }
        self.phase = Phase::Closed;
        vec![
            LinkOut::Send(Frame::Bye(omg_protocol::Bye {
                reason: reason.into(),
            })),
            LinkOut::Closed(Ok(())),
        ]
    }

    fn fail(&mut self, out: &mut Vec<LinkOut>, why: LinkEnd) {
        self.phase = Phase::Closed;
        out.push(LinkOut::Closed(Err(why)));
    }

    fn on_hello(&mut self, frame: Frame, out: &mut Vec<LinkOut>) {
        let peer = match frame {
            Frame::Hello(h) => h,
            Frame::Error(e) => return self.peer_error(out, e),
            other => return self.unexpected(out, "Hello", &other),
        };
        if peer.protocol_version != PROTOCOL_VERSION {
            out.push(LinkOut::Send(Frame::Error(ErrorMsg {
                code: ErrorCode::VersionMismatch,
                message: format!("peer v{} != v{}", peer.protocol_version, PROTOCOL_VERSION),
            })));
            return self.fail(
                out,
                LinkEnd::VersionMismatch {
                    ours: PROTOCOL_VERSION,
                    theirs: peer.protocol_version,
                },
            );
        }
        // Refused now, before any bond is stored: there is no fallback to downgrade to.
        if !peer_supports_crypto(&peer) {
            out.push(LinkOut::Send(refuse_unencrypted_peer()));
            return self.fail(out, LinkEnd::PeerNotEncrypted);
        }
        match &self.bonded {
            Some(rec) if rec.peer_pub == peer.identity_pub => self.enter_auth(peer, out),
            Some(_) => {
                out.push(LinkOut::Send(Frame::Error(ErrorMsg {
                    code: ErrorCode::UnknownPeer,
                    message: "already bonded to another Glass".into(),
                })));
                self.fail(out, LinkEnd::UnknownPeer);
            }
            None if self.mode == PairingMode::ReuseOnly => {
                out.push(LinkOut::Send(Frame::Error(ErrorMsg {
                    code: ErrorCode::NotPaired,
                    message: "no pairing and new pairing not permitted".into(),
                })));
                self.fail(out, LinkEnd::NotPaired);
            }
            None => {
                let nonce = random_nonce_b64(16);
                out.push(LinkOut::Send(Frame::PairRequest(PairRequest {
                    companion_name: self.device_name.clone(),
                    companion_pub: self.identity.public_b64(),
                    nonce: nonce.clone(),
                    bootstrap_token: None,
                })));
                self.phase = Phase::AwaitingPairAccept { nonce, peer };
            }
        }
    }

    fn on_pair_accept(&mut self, frame: Frame, nonce: String, peer: Hello, out: &mut Vec<LinkOut>) {
        let accept = match frame {
            Frame::PairAccept(a) => a,
            Frame::Error(e) => return self.peer_error(out, e),
            other => return self.unexpected(out, "PairAccept", &other),
        };
        if accept.glass_pub != peer.identity_pub {
            return self.fail(
                out,
                LinkEnd::Protocol("PairAccept key does not match Hello".into()),
            );
        }
        if accept.nonce != nonce {
            return self.fail(out, LinkEnd::Protocol("PairAccept nonce mismatch".into()));
        }
        out.push(LinkOut::Send(Frame::PairResult(PairResult {
            ok: true,
            message: "companion bonded".into(),
        })));
        let record = PairingRecord {
            peer_pub: accept.glass_pub,
            peer_name: accept.glass_name,
            bonded_at_ms: now_ms(),
        };
        self.bonded = Some(record.clone());
        out.push(LinkOut::Persist(record));
        self.enter_auth(peer, out);
    }

    fn enter_auth(&mut self, peer: Hello, out: &mut Vec<LinkOut>) {
        let nonce = self.identity.hedged_nonce_b64();
        match decode_nonce(&nonce) {
            Ok(our_nonce) => {
                out.push(LinkOut::Send(Frame::AuthChallenge(AuthChallenge { nonce })));
                self.phase = Phase::Authenticating(Auth {
                    peer,
                    our_nonce,
                    peer_nonce: None,
                    verified: None,
                });
            }
            Err(e) => self.fail(out, LinkEnd::Protocol(e.to_string())),
        }
    }

    fn on_auth(&mut self, frame: Frame, mut auth: Auth, out: &mut Vec<LinkOut>) {
        match frame {
            Frame::AuthChallenge(_) if auth.peer_nonce.is_some() => {
                return self.fail(out, LinkEnd::Protocol("a second AuthChallenge".into()));
            }
            Frame::AuthChallenge(ch) => match decode_nonce(&ch.nonce) {
                Ok(nonce) => {
                    out.push(LinkOut::Send(Frame::AuthResponse(AuthResponse {
                        signature: self.identity.sign_challenge(&nonce),
                    })));
                    auth.peer_nonce = Some(nonce);
                }
                Err(e) => return self.fail(out, LinkEnd::Protocol(e.to_string())),
            },
            Frame::AuthResponse(resp) => {
                // A response before the bond is known cannot happen: auth starts after it.
                let ok = self.bonded.as_ref().is_some_and(|b| {
                    verify_challenge(&b.peer_pub, &auth.our_nonce, &resp.signature)
                });
                out.push(LinkOut::Send(Frame::AuthResult(AuthResult {
                    ok,
                    message: if ok { "ok" } else { "bad signature" }.into(),
                })));
                if !ok {
                    return self.fail(out, LinkEnd::AuthFailed);
                }
                auth.verified = Some(ok);
            }
            Frame::AuthResult(r) => {
                if !r.ok {
                    return self.fail(out, LinkEnd::AuthFailed);
                }
                let Some(peer_nonce) = auth.peer_nonce.filter(|_| auth.verified == Some(true))
                else {
                    return self.fail(
                        out,
                        LinkEnd::Protocol("AuthResult before both sides answered".into()),
                    );
                };
                return self.enter_key_exchange(auth.peer, &auth.our_nonce, &peer_nonce, out);
            }
            Frame::Error(e) => return self.peer_error(out, e),
            other => return self.unexpected(out, "an auth frame", &other),
        }
        self.phase = Phase::Authenticating(auth);
    }

    fn enter_key_exchange(
        &mut self,
        peer: Hello,
        our_nonce: &[u8],
        peer_nonce: &[u8],
        out: &mut Vec<LinkOut>,
    ) {
        let (Some(our_hello), Some(bonded)) = (&self.our_hello, &self.bonded) else {
            return self.fail(out, LinkEnd::Protocol("key exchange before hello".into()));
        };
        let started = KeyExchange::start(
            &self.identity,
            HandshakeFacts {
                our_hello,
                peer_hello: &peer,
                our_nonce,
                peer_nonce,
                bonded_peer_pub: &bonded.peer_pub,
            },
        );
        match started {
            Ok((exchange, share)) => {
                out.push(LinkOut::Send(share));
                self.phase = Phase::AwaitingKeyShare { exchange, peer };
            }
            Err(e) => self.fail(out, LinkEnd::Protocol(e.to_string())),
        }
    }

    fn on_key_share(
        &mut self,
        frame: Frame,
        mut exchange: KeyExchange,
        peer: Hello,
        out: &mut Vec<LinkOut>,
    ) {
        let share = match frame {
            Frame::SessionKeyShare(share) => share,
            Frame::Error(e) => return self.peer_error(out, e),
            other => return self.unexpected(out, "SessionKeyShare", &other),
        };
        match exchange.on_share(&share) {
            Ok(proof) => {
                out.push(LinkOut::Send(proof));
                self.phase = Phase::AwaitingKeyProof { exchange, peer };
            }
            Err(e) => self.fail(out, LinkEnd::Protocol(e.to_string())),
        }
    }

    fn on_key_proof(
        &mut self,
        frame: Frame,
        exchange: KeyExchange,
        peer: Hello,
        out: &mut Vec<LinkOut>,
    ) {
        let proof = match frame {
            Frame::SessionKeyProof(proof) => proof,
            Frame::Error(e) => return self.peer_error(out, e),
            other => return self.unexpected(out, "SessionKeyProof", &other),
        };
        match exchange.on_proof(&proof) {
            Ok(channel) => {
                self.channel = Some(channel);
                self.phase = Phase::Running;
                out.push(LinkOut::Running {
                    peer_name: peer.device_name,
                    capabilities: peer.capabilities,
                });
            }
            Err(SecureError::BadProof) => {
                out.push(LinkOut::Send(Frame::Error(ErrorMsg {
                    code: ErrorCode::AuthFailed,
                    message: "the key exchange proof did not verify".into(),
                })));
                self.fail(out, LinkEnd::AuthFailed);
            }
            Err(e) => self.fail(out, LinkEnd::Protocol(e.to_string())),
        }
    }

    fn on_running(&mut self, frame: Frame, out: &mut Vec<LinkOut>) {
        // Nothing but the peer's next sealed frame counts from here on. Nothing is sent back: the
        // peer gets no help finding out what went wrong.
        let Some(channel) = &mut self.channel else {
            return self.fail(out, LinkEnd::Protocol("running without keys".into()));
        };
        let frame = match channel.open(frame) {
            Ok(frame) => frame,
            Err(_) => return self.fail(out, LinkEnd::Tampered),
        };
        match frame {
            Frame::Ping(p) => {
                out.push(LinkOut::Send(Frame::Pong(Pong {
                    token: p.token,
                    unix_ms: now_ms(),
                })));
                self.phase = Phase::Running;
            }
            Frame::Bye(_) => self.fail_clean(out),
            Frame::Error(e) => self.peer_error(out, e),
            other => {
                out.push(LinkOut::App(other));
                self.phase = Phase::Running;
            }
        }
    }

    fn fail_clean(&mut self, out: &mut Vec<LinkOut>) {
        self.phase = Phase::Closed;
        out.push(LinkOut::Closed(Ok(())));
    }

    fn peer_error(&mut self, out: &mut Vec<LinkOut>, e: ErrorMsg) {
        let why = match e.code {
            ErrorCode::UnknownPeer => LinkEnd::UnknownPeer,
            ErrorCode::NotPaired => LinkEnd::NotPaired,
            _ => LinkEnd::Protocol(format!("{:?}: {}", e.code, e.message)),
        };
        self.fail(out, why);
    }

    fn unexpected(&mut self, out: &mut Vec<LinkOut>, wanted: &str, got: &Frame) {
        self.fail(
            out,
            LinkEnd::Protocol(format!("expected {wanted}, got {}", got.wire_name())),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omg_protocol::{capability, Ping};

    fn glass() -> Identity {
        Identity::generate()
    }

    fn link(bonded: Option<PairingRecord>, mode: PairingMode) -> CompanionLink {
        CompanionLink::new(
            Identity::generate(),
            bonded,
            mode,
            "phone",
            "0",
            vec![capability::NOTES_SYNC.to_string()],
        )
    }

    fn hello(identity: &Identity) -> Frame {
        Frame::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            role: Role::Glass,
            app_version: "0".into(),
            device_name: "glass".into(),
            identity_pub: identity.public_b64(),
            capabilities: vec![
                capability::NOTES_SYNC.to_string(),
                capability::SESSION_CRYPTO.to_string(),
            ],
        })
    }

    fn sent(out: &[LinkOut]) -> Vec<&Frame> {
        out.iter()
            .filter_map(|o| match o {
                LinkOut::Send(f) => Some(f),
                _ => None,
            })
            .collect()
    }

    /// Plays the Glass through the key exchange, from a link that verified the peer's signature.
    /// Returns the Glass's end of the encrypted link.
    fn finish_handshake(l: &mut CompanionLink, g: &Identity, nonce_c: Vec<u8>) -> Channel {
        let our_hello = l.our_hello.clone().unwrap();
        let Frame::Hello(g_hello) = hello(g) else {
            panic!();
        };
        let nonce_g = decode_nonce(&random_nonce_b64(32)).unwrap();
        let out = l.on_frame(Frame::AuthChallenge(AuthChallenge {
            nonce: omg_base64(&nonce_g),
        }));
        assert!(matches!(sent(&out)[0], Frame::AuthResponse(_)));
        let out = l.on_frame(Frame::AuthResult(AuthResult {
            ok: true,
            message: "ok".into(),
        }));
        let Frame::SessionKeyShare(c_share) = sent(&out)[0].clone() else {
            panic!("no share");
        };
        assert!(!l.is_running(), "auth alone must not start the link");
        let (mut exchange, g_share) = KeyExchange::start(
            g,
            HandshakeFacts {
                our_hello: &g_hello,
                peer_hello: &our_hello,
                our_nonce: &nonce_g,
                peer_nonce: &nonce_c,
                bonded_peer_pub: &l.identity.public_b64(),
            },
        )
        .unwrap();
        let g_proof = exchange.on_share(&c_share).unwrap();
        let out = l.on_frame(g_share);
        let Frame::SessionKeyProof(c_proof) = sent(&out)[0].clone() else {
            panic!("no proof");
        };
        assert!(!l.is_running(), "the share alone must not start the link");
        let channel = exchange.on_proof(&c_proof).unwrap();
        let out = l.on_frame(g_proof);
        assert!(matches!(out[0], LinkOut::Running { .. }));
        assert!(l.is_running());
        channel
    }

    fn omg_base64(bytes: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn pair(glass: &Identity) -> (CompanionLink, Vec<u8>) {
        let mut l = link(None, PairingMode::AllowNew);
        l.start();
        let out = l.on_frame(hello(glass));
        let Frame::PairRequest(req) = sent(&out)[0].clone() else {
            panic!("no pair request");
        };
        let out = l.on_frame(Frame::PairAccept(omg_protocol::PairAccept {
            glass_name: "glass".into(),
            glass_pub: glass.public_b64(),
            nonce: req.nonce,
        }));
        assert!(out.iter().any(|o| matches!(o, LinkOut::Persist(_))));
        let challenge = sent(&out)
            .iter()
            .find_map(|f| match f {
                Frame::AuthChallenge(c) => Some(c.nonce.clone()),
                _ => None,
            })
            .expect("a challenge");
        (l, decode_nonce(&challenge).unwrap())
    }

    #[test]
    fn pairing_then_mutual_auth_reaches_running() {
        let g = glass();
        let (mut l, nonce) = pair(&g);
        let out = l.on_frame(Frame::AuthResponse(AuthResponse {
            signature: g.sign_challenge(&nonce),
        }));
        assert!(matches!(
            sent(&out)[0],
            Frame::AuthResult(AuthResult { ok: true, .. })
        ));
        let mut glass_end = finish_handshake(&mut l, &g, nonce);

        // Pings are answered here; other frames go up.
        let ping = glass_end
            .seal(&Frame::Ping(Ping {
                token: 9,
                unix_ms: 0,
            }))
            .unwrap();
        let out = l.on_frame(ping);
        assert!(matches!(sent(&out)[0], Frame::Pong(Pong { token: 9, .. })));
        let out = l.on_frame(glass_end.seal(&Frame::SettingsGet).unwrap());
        assert!(matches!(out[0], LinkOut::App(Frame::SettingsGet)));

        // What goes out is sealed: the Glass's end opens it.
        let bytes = l.encode(&Frame::SettingsGet).unwrap();
        let mut decoder = omg_protocol::FrameDecoder::new();
        decoder.push(&bytes);
        let outer = decoder.next_frame().unwrap().unwrap();
        assert!(matches!(outer, Frame::Sealed(_)));
        assert_eq!(glass_end.open(outer).unwrap(), Frame::SettingsGet);
    }

    #[test]
    fn a_frame_in_clear_on_a_running_link_ends_it() {
        let g = glass();
        let (mut l, nonce) = pair(&g);
        l.on_frame(Frame::AuthResponse(AuthResponse {
            signature: g.sign_challenge(&nonce),
        }));
        finish_handshake(&mut l, &g, nonce);
        let out = l.on_frame(Frame::Ping(Ping {
            token: 1,
            unix_ms: 0,
        }));
        assert!(out
            .iter()
            .any(|o| matches!(o, LinkOut::Closed(Err(LinkEnd::Tampered)))));
        assert!(!out.iter().any(|o| matches!(o, LinkOut::Send(_))));
        assert!(l.is_closed());
    }

    #[test]
    fn application_frames_cannot_leave_before_the_keys_exist() {
        let g = glass();
        let (mut l, _) = pair(&g);
        assert!(matches!(
            l.encode(&Frame::SettingsGet),
            Err(SecureError::NotYet(_))
        ));
    }

    #[test]
    fn a_peer_that_does_not_encrypt_is_refused_before_anything_is_stored() {
        let mut l = link(None, PairingMode::AllowNew);
        l.start();
        let Frame::Hello(mut h) = hello(&glass()) else {
            panic!();
        };
        h.capabilities.retain(|c| c != capability::SESSION_CRYPTO);
        let out = l.on_frame(Frame::Hello(h));
        assert!(out
            .iter()
            .any(|o| matches!(o, LinkOut::Closed(Err(LinkEnd::PeerNotEncrypted)))));
        assert!(!out.iter().any(|o| matches!(o, LinkOut::Persist(_))));
        assert!(!sent(&out)
            .iter()
            .any(|f| matches!(f, Frame::PairRequest(_) | Frame::AuthChallenge(_))));
    }

    #[test]
    fn a_wrong_signature_ends_the_link() {
        let g = glass();
        let (mut l, nonce) = pair(&g);
        let out = l.on_frame(Frame::AuthResponse(AuthResponse {
            signature: glass().sign_challenge(&nonce),
        }));
        assert!(out
            .iter()
            .any(|o| matches!(o, LinkOut::Closed(Err(LinkEnd::AuthFailed)))));
        assert!(l.is_closed());
    }

    #[test]
    fn running_cannot_be_reached_without_verifying_the_peer() {
        let g = glass();
        let (mut l, _) = pair(&g);
        let out = l.on_frame(Frame::AuthResult(AuthResult {
            ok: true,
            message: "ok".into(),
        }));
        assert!(!out.iter().any(|o| matches!(o, LinkOut::Running { .. })));
        assert!(l.is_closed());
    }

    #[test]
    fn a_different_key_than_the_bonded_one_is_refused() {
        let bond = PairingRecord {
            peer_pub: glass().public_b64(),
            peer_name: "old".into(),
            bonded_at_ms: 0,
        };
        let mut l = link(Some(bond), PairingMode::AllowNew);
        l.start();
        let out = l.on_frame(hello(&glass()));
        assert!(out
            .iter()
            .any(|o| matches!(o, LinkOut::Closed(Err(LinkEnd::UnknownPeer)))));
    }

    #[test]
    fn no_bond_without_permission_is_refused() {
        let mut l = link(None, PairingMode::ReuseOnly);
        l.start();
        let out = l.on_frame(hello(&glass()));
        assert!(out
            .iter()
            .any(|o| matches!(o, LinkOut::Closed(Err(LinkEnd::NotPaired)))));
    }
}
