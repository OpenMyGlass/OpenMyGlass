//! The companion's side of the handshake as a pure state machine.
//!
//! A frontend that owns its socket feeds it decoded frames and carries out what comes back.
//! It does the same as `session.rs`, which runs both roles as an async flow for the CLI and the
//! simulated Glass; `tests/companion_link.rs` runs this one against that one over TCP, so the
//! two cannot drift apart unnoticed.
//!
//! Nothing here depends on the kind of link. Over Bluetooth or Wi-Fi alike, the peer is proved
//! by its bonded ed25519 key.

use omg_protocol::{
    AuthChallenge, AuthResponse, AuthResult, ErrorCode, ErrorMsg, Frame, Hello, PairRequest,
    PairResult, Pong, Role, PROTOCOL_VERSION,
};

use crate::{
    identity::{decode_nonce, random_nonce_b64, verify_challenge, Identity},
    now_ms,
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
    /// The peer is authenticated.
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
    Running,
    Closed,
}

#[derive(Debug)]
struct Auth {
    peer: Hello,
    our_nonce: Vec<u8>,
    answered_peer: bool,
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
            capabilities,
            phase: Phase::Start,
        }
    }

    /// Whether the peer is authenticated.
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
        vec![LinkOut::Send(Frame::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            role: Role::Companion,
            app_version: self.app_version.clone(),
            device_name: self.device_name.clone(),
            identity_pub: self.identity.public_b64(),
            capabilities: self.capabilities.clone(),
        }))]
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
        let nonce = random_nonce_b64(32);
        match decode_nonce(&nonce) {
            Ok(our_nonce) => {
                out.push(LinkOut::Send(Frame::AuthChallenge(AuthChallenge { nonce })));
                self.phase = Phase::Authenticating(Auth {
                    peer,
                    our_nonce,
                    answered_peer: false,
                    verified: None,
                });
            }
            Err(e) => self.fail(out, LinkEnd::Protocol(e.to_string())),
        }
    }

    fn on_auth(&mut self, frame: Frame, mut auth: Auth, out: &mut Vec<LinkOut>) {
        match frame {
            Frame::AuthChallenge(ch) => match decode_nonce(&ch.nonce) {
                Ok(nonce) => {
                    out.push(LinkOut::Send(Frame::AuthResponse(AuthResponse {
                        signature: self.identity.sign_challenge(&nonce),
                    })));
                    auth.answered_peer = true;
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
                if !(auth.answered_peer && auth.verified == Some(true)) {
                    return self.fail(
                        out,
                        LinkEnd::Protocol("AuthResult before both sides answered".into()),
                    );
                }
                out.push(LinkOut::Running {
                    peer_name: auth.peer.device_name,
                    capabilities: auth.peer.capabilities,
                });
                self.phase = Phase::Running;
                return;
            }
            Frame::Error(e) => return self.peer_error(out, e),
            other => return self.unexpected(out, "an auth frame", &other),
        }
        self.phase = Phase::Authenticating(auth);
    }

    fn on_running(&mut self, frame: Frame, out: &mut Vec<LinkOut>) {
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
            capabilities: vec![capability::NOTES_SYNC.to_string()],
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
        l.on_frame(Frame::AuthChallenge(AuthChallenge {
            nonce: random_nonce_b64(32),
        }));
        let out = l.on_frame(Frame::AuthResult(AuthResult {
            ok: true,
            message: "ok".into(),
        }));
        assert!(matches!(out[0], LinkOut::Running { .. }));
        assert!(l.is_running());

        // Pings are answered here; other frames go up.
        let out = l.on_frame(Frame::Ping(Ping {
            token: 9,
            unix_ms: 0,
        }));
        assert!(matches!(sent(&out)[0], Frame::Pong(Pong { token: 9, .. })));
        let out = l.on_frame(Frame::SettingsGet);
        assert!(matches!(out[0], LinkOut::App(Frame::SettingsGet)));
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
