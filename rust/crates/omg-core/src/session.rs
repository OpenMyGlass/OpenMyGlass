//! The link session state machine: Hello exchange, authorize/pair, mutual auth, then
//! the running message loop. This is fully implemented and exercised end-to-end by the
//! integration tests over both an in-memory pipe and a real TCP socket.

use omg_protocol::{
    capability, AuthChallenge, AuthResponse, AuthResult, Bye, ErrorCode, ErrorMsg, Frame, Hello,
    PairAccept, PairRequest, PairResult, Ping, Pong, Role, PROTOCOL_VERSION,
};
use omg_transport::{Link, Stream};
use tracing::{debug, info, warn};

use crate::{
    identity::{decode_nonce, random_nonce_b64, verify_challenge, Identity},
    now_ms,
    store::PairingRecord,
    CoreError,
};

/// How a session should treat an unbonded peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingMode {
    /// Only talk to the already-bonded peer; refuse to bond a new one.
    ReuseOnly,
    /// Permit a one-time first-pairing with a new peer (user initiated "pair" action).
    AllowNew,
}

/// Static per-session configuration.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Which side we are.
    pub role: Role,
    /// Friendly local device name.
    pub device_name: String,
    /// Our build version string.
    pub app_version: String,
    /// Capabilities we advertise.
    pub capabilities: Vec<String>,
}

impl SessionConfig {
    /// A companion with the backbone capabilities.
    pub fn companion(device_name: impl Into<String>) -> Self {
        SessionConfig {
            role: Role::Companion,
            device_name: device_name.into(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            capabilities: capability::BACKBONE.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// A (simulated or real) Glass with the backbone capabilities.
    pub fn glass(device_name: impl Into<String>) -> Self {
        SessionConfig {
            role: Role::Glass,
            device_name: device_name.into(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            capabilities: capability::BACKBONE.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// An established (or establishing) link session.
pub struct Session<S> {
    link: Link<S>,
    identity: Identity,
    bonded: Option<PairingRecord>,
    cfg: SessionConfig,
    peer_hello: Option<Hello>,
}

impl<S: Stream> Session<S> {
    /// Wrap a link. `bonded` is the currently-remembered peer (from the store), if any.
    pub fn new(
        link: Link<S>,
        identity: Identity,
        bonded: Option<PairingRecord>,
        cfg: SessionConfig,
    ) -> Self {
        Session {
            link,
            identity,
            bonded,
            cfg,
            peer_hello: None,
        }
    }

    /// Add a capability to advertise in our Hello. Call before the handshake.
    pub fn advertise(&mut self, capability: &str) {
        if !self.cfg.capabilities.iter().any(|c| c == capability) {
            self.cfg.capabilities.push(capability.to_string());
        }
    }

    /// Whether the peer advertised this capability (false before the Hello exchange).
    pub fn peer_supports(&self, capability: &str) -> bool {
        self.peer_hello
            .as_ref()
            .is_some_and(|h| h.capabilities.iter().any(|c| c == capability))
    }

    /// Give up the session after the handshake, keeping the link.
    pub fn into_link(self) -> Link<S> {
        self.link
    }

    /// The peer's Hello (available after [`exchange_hello`](Session::exchange_hello)).
    pub fn peer_hello(&self) -> Option<&Hello> {
        self.peer_hello.as_ref()
    }

    /// The bonded peer after a successful [`authorize`](Session::authorize)/pairing.
    pub fn bonded(&self) -> Option<&PairingRecord> {
        self.bonded.as_ref()
    }

    /// Exchange Hello frames. We send ours, then read the peer's (which must be the
    /// first frame it sends).
    pub async fn exchange_hello(&mut self) -> Result<Hello, CoreError> {
        let hello = Hello {
            protocol_version: PROTOCOL_VERSION,
            role: self.cfg.role,
            app_version: self.cfg.app_version.clone(),
            device_name: self.cfg.device_name.clone(),
            identity_pub: self.identity.public_b64(),
            capabilities: self.cfg.capabilities.clone(),
        };
        self.link.send(Frame::Hello(hello)).await?;

        let frame = self.recv_required().await?;
        let peer = match frame {
            Frame::Hello(h) => h,
            other => {
                return Err(CoreError::Protocol(format!(
                    "expected Hello, got {}",
                    other.wire_name()
                )))
            }
        };
        if peer.protocol_version != PROTOCOL_VERSION {
            let _ = self
                .link
                .send(Frame::Error(ErrorMsg {
                    code: ErrorCode::VersionMismatch,
                    message: format!("peer v{} != v{}", peer.protocol_version, PROTOCOL_VERSION),
                }))
                .await;
            return Err(CoreError::VersionMismatch {
                ours: PROTOCOL_VERSION,
                theirs: peer.protocol_version,
            });
        }
        debug!(role = ?peer.role, name = %peer.device_name, "peer hello");
        self.peer_hello = Some(peer.clone());
        Ok(peer)
    }

    /// Decide whether to talk to this peer, pairing a new one if `mode` allows.
    ///
    /// * Already bonded + same key → accept (persistent re-use).
    /// * Already bonded + different key → **reject** (one companion only).
    /// * Not bonded + `AllowNew` → run the one-time pairing handshake.
    /// * Not bonded + `ReuseOnly` → reject.
    ///
    /// On success `self.bonded` is set; the caller persists it via the store.
    pub async fn authorize(&mut self, mode: PairingMode) -> Result<PairingRecord, CoreError> {
        let peer = self
            .peer_hello
            .clone()
            .ok_or_else(|| CoreError::Protocol("authorize before hello".into()))?;
        let peer_pub = peer.identity_pub.clone();

        if let Some(rec) = self.bonded.clone() {
            if rec.peer_pub == peer_pub {
                debug!("known bonded peer");
                return Ok(rec);
            }
            // A different device is trying to use an already-bonded side.
            let _ = self
                .link
                .send(Frame::Error(ErrorMsg {
                    code: ErrorCode::UnknownPeer,
                    message: "already bonded to another companion".into(),
                }))
                .await;
            return Err(CoreError::UnknownPeer);
        }

        if mode == PairingMode::ReuseOnly {
            let _ = self
                .link
                .send(Frame::Error(ErrorMsg {
                    code: ErrorCode::NotPaired,
                    message: "no pairing and new pairing not permitted".into(),
                }))
                .await;
            return Err(CoreError::NotPaired);
        }

        let rec = match self.cfg.role {
            Role::Companion => self.pair_as_companion(&peer).await?,
            Role::Glass => self.pair_as_glass(&peer).await?,
        };
        info!(peer = %rec.peer_name, "paired");
        self.bonded = Some(rec.clone());
        Ok(rec)
    }

    async fn pair_as_companion(&mut self, peer: &Hello) -> Result<PairingRecord, CoreError> {
        let nonce = random_nonce_b64(16);
        self.link
            .send(Frame::PairRequest(PairRequest {
                companion_name: self.cfg.device_name.clone(),
                companion_pub: self.identity.public_b64(),
                nonce: nonce.clone(),
                bootstrap_token: None,
            }))
            .await?;

        let accept = match self.recv_required().await? {
            Frame::PairAccept(a) => a,
            Frame::Error(e) => return Err(CoreError::PeerError(e)),
            other => {
                return Err(CoreError::Protocol(format!(
                    "expected PairAccept, got {}",
                    other.wire_name()
                )))
            }
        };
        if accept.glass_pub != peer.identity_pub {
            return Err(CoreError::Protocol(
                "PairAccept key does not match Hello".into(),
            ));
        }
        if accept.nonce != nonce {
            return Err(CoreError::Protocol("PairAccept nonce mismatch".into()));
        }
        self.link
            .send(Frame::PairResult(PairResult {
                ok: true,
                message: "companion bonded".into(),
            }))
            .await?;
        Ok(PairingRecord {
            peer_pub: accept.glass_pub,
            peer_name: accept.glass_name,
            bonded_at_ms: now_ms(),
        })
    }

    async fn pair_as_glass(&mut self, _peer: &Hello) -> Result<PairingRecord, CoreError> {
        let req = match self.recv_required().await? {
            Frame::PairRequest(r) => r,
            Frame::Error(e) => return Err(CoreError::PeerError(e)),
            other => {
                return Err(CoreError::Protocol(format!(
                    "expected PairRequest, got {}",
                    other.wire_name()
                )))
            }
        };
        self.link
            .send(Frame::PairAccept(PairAccept {
                glass_name: self.cfg.device_name.clone(),
                glass_pub: self.identity.public_b64(),
                nonce: req.nonce.clone(),
            }))
            .await?;
        // Companion confirms.
        match self.recv_required().await? {
            Frame::PairResult(r) if r.ok => {}
            Frame::PairResult(r) => {
                return Err(CoreError::Protocol(format!(
                    "pairing refused: {}",
                    r.message
                )))
            }
            other => {
                return Err(CoreError::Protocol(format!(
                    "expected PairResult, got {}",
                    other.wire_name()
                )))
            }
        }
        Ok(PairingRecord {
            peer_pub: req.companion_pub,
            peer_name: req.companion_name,
            bonded_at_ms: now_ms(),
        })
    }

    /// Mutual challenge/response: each side proves it holds the private key behind the
    /// bonded public key. Must be called after [`authorize`](Session::authorize).
    pub async fn authenticate(&mut self) -> Result<(), CoreError> {
        let bonded = self
            .bonded
            .clone()
            .ok_or_else(|| CoreError::Protocol("authenticate before authorize".into()))?;

        // Send our challenge first; both sides' challenges are then in flight.
        let our_nonce_b64 = random_nonce_b64(32);
        let our_nonce = decode_nonce(&our_nonce_b64)?;
        self.link
            .send(Frame::AuthChallenge(AuthChallenge {
                nonce: our_nonce_b64,
            }))
            .await?;

        let mut answered_peer = false; // we signed their challenge
        let mut got_their_response = false; // we verified their signature
        let mut verify_ok = false;

        while !(answered_peer && got_their_response) {
            match self.recv_required().await? {
                Frame::AuthChallenge(ch) => {
                    let nonce = decode_nonce(&ch.nonce)?;
                    let sig = self.identity.sign_challenge(&nonce);
                    self.link
                        .send(Frame::AuthResponse(AuthResponse { signature: sig }))
                        .await?;
                    answered_peer = true;
                }
                Frame::AuthResponse(resp) => {
                    verify_ok = verify_challenge(&bonded.peer_pub, &our_nonce, &resp.signature);
                    got_their_response = true;
                }
                Frame::Error(e) => return Err(CoreError::PeerError(e)),
                other => {
                    return Err(CoreError::Protocol(format!(
                        "unexpected {} during auth",
                        other.wire_name()
                    )))
                }
            }
        }

        self.link
            .send(Frame::AuthResult(AuthResult {
                ok: verify_ok,
                message: if verify_ok {
                    "ok".into()
                } else {
                    "bad signature".into()
                },
            }))
            .await?;
        if !verify_ok {
            return Err(CoreError::AuthFailed);
        }

        match self.recv_required().await? {
            Frame::AuthResult(r) if r.ok => {
                info!("mutual auth ok");
                Ok(())
            }
            Frame::AuthResult(_) => Err(CoreError::AuthFailed),
            other => Err(CoreError::Protocol(format!(
                "expected AuthResult, got {}",
                other.wire_name()
            ))),
        }
    }

    /// Run the full companion-side handshake: hello, authorize/pair, auth.
    /// Returns the bonded peer (persist it in the store with its key/name).
    pub async fn handshake(&mut self, mode: PairingMode) -> Result<PairingRecord, CoreError> {
        self.exchange_hello().await?;
        let rec = self.authorize(mode).await?;
        self.authenticate().await?;
        Ok(rec)
    }

    /// Send any frame.
    pub async fn send(&mut self, frame: Frame) -> Result<(), CoreError> {
        Ok(self.link.send(frame).await?)
    }

    /// Receive the next application frame. [`Ping`] is answered with [`Pong`]
    /// transparently and not returned; `None` means the peer closed the link.
    pub async fn recv_app(&mut self) -> Result<Option<Frame>, CoreError> {
        loop {
            match self.link.recv().await? {
                None => return Ok(None),
                Some(Frame::Ping(p)) => {
                    self.link
                        .send(Frame::Pong(Pong {
                            token: p.token,
                            unix_ms: now_ms(),
                        }))
                        .await?;
                }
                Some(Frame::Bye(b)) => {
                    debug!(reason = %b.reason, "peer said bye");
                    return Ok(None);
                }
                Some(other) => return Ok(Some(other)),
            }
        }
    }

    /// Send a [`Ping`] and wait for the matching [`Pong`], returning the round-trip in
    /// milliseconds. Other frames arriving meanwhile are handled/ignored.
    pub async fn heartbeat(&mut self) -> Result<u64, CoreError> {
        let token = now_ms();
        let sent = now_ms();
        self.link
            .send(Frame::Ping(Ping {
                token,
                unix_ms: sent,
            }))
            .await?;
        loop {
            match self.link.recv().await? {
                None => return Err(CoreError::UnexpectedEof),
                Some(Frame::Pong(p)) if p.token == token => {
                    return Ok(now_ms().saturating_sub(sent));
                }
                Some(Frame::Ping(p)) => {
                    self.link
                        .send(Frame::Pong(Pong {
                            token: p.token,
                            unix_ms: now_ms(),
                        }))
                        .await?;
                }
                Some(other) => {
                    warn!(
                        frame = other.wire_name(),
                        "ignoring frame while awaiting pong"
                    );
                }
            }
        }
    }

    /// Send a graceful [`Bye`] and stop.
    pub async fn close(&mut self, reason: impl Into<String>) -> Result<(), CoreError> {
        self.link
            .send(Frame::Bye(Bye {
                reason: reason.into(),
            }))
            .await?;
        Ok(())
    }

    async fn recv_required(&mut self) -> Result<Frame, CoreError> {
        self.link.recv().await?.ok_or(CoreError::UnexpectedEof)
    }
}
