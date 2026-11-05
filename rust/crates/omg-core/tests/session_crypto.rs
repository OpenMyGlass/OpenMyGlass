//! What a relay in the middle of the link can do once the session is encrypted: nothing but cut it.
//! Each test puts a frame-aware relay between two real [`Session`]s and sees every byte.

use std::sync::{Arc, Mutex};

use omg_core::{
    identity::Identity,
    secure::SecureError,
    session::{PairingMode, Session, SessionConfig},
    CoreError, PairingRecord,
};
use omg_protocol::{
    capability, frame_bytes, ErrorCode, Frame, FrameDecoder, Hello, Role, SessionKeyShare,
    WifiBringUpResult, PROTOCOL_VERSION,
};
use omg_transport::Link;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};

const MARKER: &str = "MARKER-AFTER-AUTH-hunter2";

fn secret_frame() -> Frame {
    Frame::WifiBringUpResult(WifiBringUpResult {
        ok: true,
        ssid: Some(MARKER.into()),
        ip: None,
        message: String::new(),
    })
}

fn bond(peer: &Identity, name: &str) -> PairingRecord {
    PairingRecord {
        peer_pub: peer.public_b64(),
        peer_name: name.into(),
        bonded_at_ms: 0,
    }
}

type Log = Arc<Mutex<Vec<u8>>>;

/// Copies frames one way, letting `policy` decide what the other side gets for each.
async fn pipe(
    mut from: ReadHalf<DuplexStream>,
    mut to: WriteHalf<DuplexStream>,
    log: Log,
    mut policy: impl FnMut(Frame) -> Vec<Frame>,
) {
    let mut decoder = FrameDecoder::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = from.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            let _ = to.shutdown().await;
            return;
        }
        log.lock().unwrap().extend_from_slice(&buf[..n]);
        decoder.push(&buf[..n]);
        while let Some(frame) = decoder.next_frame().unwrap() {
            for out in policy(frame) {
                if to.write_all(&frame_bytes(&out).unwrap()).await.is_err() {
                    return;
                }
            }
        }
    }
}

struct Peers {
    companion: Session<DuplexStream>,
    glass: Session<DuplexStream>,
    log: Log,
}

/// Two bonded sessions and a relay in between, with one policy per direction.
fn relayed(
    to_glass: impl FnMut(Frame) -> Vec<Frame> + Send + 'static,
    to_companion: impl FnMut(Frame) -> Vec<Frame> + Send + 'static,
) -> Peers {
    let (c, c_relay) = tokio::io::duplex(1 << 16);
    let (g, g_relay) = tokio::io::duplex(1 << 16);
    let (c_read, c_write) = tokio::io::split(c_relay);
    let (g_read, g_write) = tokio::io::split(g_relay);
    let log = Log::default();
    tokio::spawn(pipe(c_read, g_write, log.clone(), to_glass));
    tokio::spawn(pipe(g_read, c_write, log.clone(), to_companion));
    let (c_id, g_id) = (Identity::generate(), Identity::generate());
    Peers {
        companion: Session::new(
            Link::new(c),
            c_id.clone(),
            Some(bond(&g_id, "glass")),
            SessionConfig::companion("phone"),
        ),
        glass: Session::new(
            Link::new(g),
            g_id,
            Some(bond(&c_id, "phone")),
            SessionConfig::glass("glass"),
        ),
        log,
    }
}

fn forward(frame: Frame) -> Vec<Frame> {
    vec![frame]
}

async fn handshake(p: &mut Peers) {
    let (a, b) = tokio::join!(
        p.companion.handshake(PairingMode::ReuseOnly),
        p.glass.handshake(PairingMode::ReuseOnly)
    );
    a.unwrap();
    b.unwrap();
}

/// A policy that treats sealed frames by their number, from 0, and passes the rest.
fn on_sealed(
    mut f: impl FnMut(usize, Frame) -> Vec<Frame> + Send + 'static,
) -> impl FnMut(Frame) -> Vec<Frame> + Send + 'static {
    let mut n = 0;
    move |frame| {
        if matches!(frame, Frame::Sealed(_)) {
            n += 1;
            f(n - 1, frame)
        } else {
            vec![frame]
        }
    }
}

fn is_rejection<T: std::fmt::Debug>(r: Result<T, CoreError>) -> bool {
    matches!(
        r,
        Err(CoreError::Transport(omg_transport::TransportError::Wire(
            omg_protocol::WireError::Rejected(_)
        )))
    )
}

#[tokio::test]
async fn a_relay_reads_nothing_after_the_handshake_in_either_direction() {
    let mut p = relayed(forward, forward);
    handshake(&mut p).await;
    p.companion.send(secret_frame()).await.unwrap();
    assert_eq!(p.glass.recv_app().await.unwrap().unwrap(), secret_frame());
    p.glass.send(secret_frame()).await.unwrap();
    assert_eq!(
        p.companion.recv_app().await.unwrap().unwrap(),
        secret_frame()
    );
    let log = p.log.lock().unwrap();
    let seen = |needle: &[u8]| log.windows(needle.len()).any(|w| w == needle);
    assert!(!seen(MARKER.as_bytes()), "the marker crossed in clear");
    // The relay did see the link: the handshake is in clear, and that is where the name is.
    assert!(seen(b"phone"));
}

#[tokio::test]
async fn a_frame_forged_in_clear_ends_the_link() {
    let mut p = relayed(on_sealed(|_, _| vec![Frame::WifiTearDown]), forward);
    handshake(&mut p).await;
    p.companion.send(secret_frame()).await.unwrap();
    assert!(is_rejection(p.glass.recv_app().await));
}

#[tokio::test]
async fn a_frame_forged_towards_the_companion_ends_the_link() {
    let mut p = relayed(forward, on_sealed(|_, _| vec![Frame::Unpair]));
    handshake(&mut p).await;
    p.glass.send(secret_frame()).await.unwrap();
    assert!(is_rejection(p.companion.recv_app().await));
}

#[tokio::test]
async fn a_replayed_frame_ends_the_link() {
    let mut p = relayed(on_sealed(|_, f| vec![f.clone(), f]), forward);
    handshake(&mut p).await;
    p.companion.send(secret_frame()).await.unwrap();
    assert_eq!(p.glass.recv_app().await.unwrap().unwrap(), secret_frame());
    assert!(is_rejection(p.glass.recv_app().await));
}

#[tokio::test]
async fn reordered_frames_end_the_link() {
    let mut held = None;
    let mut p = relayed(
        on_sealed(move |n, f| match n {
            0 => {
                held = Some(f);
                Vec::new()
            }
            _ => [Some(f), held.take()].into_iter().flatten().collect(),
        }),
        forward,
    );
    handshake(&mut p).await;
    p.companion.send(secret_frame()).await.unwrap();
    p.companion.send(Frame::SettingsGet).await.unwrap();
    assert!(is_rejection(p.glass.recv_app().await));
}

#[tokio::test]
async fn a_dropped_frame_ends_the_link() {
    let mut p = relayed(
        on_sealed(|n, f| if n == 0 { Vec::new() } else { vec![f] }),
        forward,
    );
    handshake(&mut p).await;
    p.companion.send(secret_frame()).await.unwrap();
    p.companion.send(Frame::SettingsGet).await.unwrap();
    assert!(is_rejection(p.glass.recv_app().await));
}

#[tokio::test]
async fn a_changed_bit_ends_the_link() {
    let mut p = relayed(
        on_sealed(|_, f| {
            let Frame::Sealed(mut s) = f else {
                unreachable!()
            };
            s.ciphertext[0] ^= 1;
            vec![Frame::Sealed(s)]
        }),
        forward,
    );
    handshake(&mut p).await;
    p.companion.send(secret_frame()).await.unwrap();
    assert!(is_rejection(p.glass.recv_app().await));
}

#[tokio::test]
async fn frames_replayed_from_another_session_end_the_link() {
    // Capture a sealed frame from one session and play it into the next, between the same two
    // identities.
    let captured: Arc<Mutex<Option<Frame>>> = Arc::default();
    let (c_id, g_id) = (Identity::generate(), Identity::generate());
    let run = |policy: Box<dyn FnMut(Frame) -> Vec<Frame> + Send>| {
        let (c, c_relay) = tokio::io::duplex(1 << 16);
        let (g, g_relay) = tokio::io::duplex(1 << 16);
        let (c_read, c_write) = tokio::io::split(c_relay);
        let (g_read, g_write) = tokio::io::split(g_relay);
        let log = Log::default();
        tokio::spawn(pipe(c_read, g_write, log.clone(), policy));
        tokio::spawn(pipe(g_read, c_write, log, forward));
        (
            Session::new(
                Link::new(c),
                c_id.clone(),
                Some(bond(&g_id, "glass")),
                SessionConfig::companion("phone"),
            ),
            Session::new(
                Link::new(g),
                g_id.clone(),
                Some(bond(&c_id, "phone")),
                SessionConfig::glass("glass"),
            ),
        )
    };
    let keep = captured.clone();
    let (mut c, mut g) = run(Box::new(on_sealed(move |_, f| {
        *keep.lock().unwrap() = Some(f.clone());
        vec![f]
    })));
    let (a, b) = tokio::join!(
        c.handshake(PairingMode::ReuseOnly),
        g.handshake(PairingMode::ReuseOnly)
    );
    a.unwrap();
    b.unwrap();
    c.send(secret_frame()).await.unwrap();
    g.recv_app().await.unwrap().unwrap();

    let old = captured.lock().unwrap().clone().unwrap();
    let (mut c, mut g) = run(Box::new(on_sealed(move |_, _| vec![old.clone()])));
    let (a, b) = tokio::join!(
        c.handshake(PairingMode::ReuseOnly),
        g.handshake(PairingMode::ReuseOnly)
    );
    a.unwrap();
    b.unwrap();
    c.send(secret_frame()).await.unwrap();
    assert!(is_rejection(g.recv_app().await));
}

/// Swaps the shares for ones the relay holds the secret of: the attack the proofs exist for. The
/// relay forwards everything else, including both real signatures over the challenges.
fn substitute_share(frame: Frame) -> Vec<Frame> {
    match frame {
        Frame::SessionKeyShare(_) => vec![Frame::SessionKeyShare(SessionKeyShare {
            ephemeral_pub: vec![9; 32],
        })],
        other => vec![other],
    }
}

#[tokio::test]
async fn a_bonded_pair_resists_a_relay_replacing_the_key_shares() {
    let mut p = relayed(substitute_share, substitute_share);
    let (a, b) = tokio::join!(
        p.companion.handshake(PairingMode::ReuseOnly),
        p.glass.handshake(PairingMode::ReuseOnly)
    );
    assert!(matches!(
        a,
        Err(CoreError::KeyExchange(SecureError::BadProof))
    ));
    assert!(matches!(
        b,
        Err(CoreError::KeyExchange(SecureError::BadProof))
    ));
}

#[tokio::test]
async fn a_relay_cannot_change_what_a_hello_says() {
    let rename = |frame| match frame {
        Frame::Hello(mut h) => {
            h.device_name = "someone else".into();
            vec![Frame::Hello(h)]
        }
        other => vec![other],
    };
    let mut p = relayed(rename, forward);
    let (a, b) = tokio::join!(
        p.companion.handshake(PairingMode::ReuseOnly),
        p.glass.handshake(PairingMode::ReuseOnly)
    );
    assert!(matches!(
        a,
        Err(CoreError::KeyExchange(SecureError::BadProof))
    ));
    assert!(matches!(
        b,
        Err(CoreError::KeyExchange(SecureError::BadProof))
    ));
}

#[tokio::test]
async fn stripping_the_capability_cannot_downgrade_the_link() {
    let strip = |frame| match frame {
        Frame::Hello(mut h) => {
            h.capabilities.retain(|c| c != capability::SESSION_CRYPTO);
            vec![Frame::Hello(h)]
        }
        other => vec![other],
    };
    let mut p = relayed(forward, strip);
    let (a, _) = tokio::join!(
        p.companion.handshake(PairingMode::ReuseOnly),
        p.glass.handshake(PairingMode::ReuseOnly)
    );
    assert!(matches!(a, Err(CoreError::PeerNotEncrypted)));
    assert!(p.companion.bonded().is_some(), "the bond is left alone");
}

#[tokio::test]
async fn a_peer_that_does_not_encrypt_is_refused_with_a_reason() {
    let (c, g) = tokio::io::duplex(1 << 16);
    let mut old = Link::new(c);
    let peer = tokio::spawn(async move {
        old.send(Frame::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            role: Role::Companion,
            app_version: "old".into(),
            device_name: "old phone".into(),
            identity_pub: Identity::generate().public_b64(),
            capabilities: vec![capability::NOTIFICATIONS.to_string()],
        }))
        .await
        .unwrap();
        // Its own Hello, then the refusal.
        let _hello = old.recv().await.unwrap().unwrap();
        old.recv().await.unwrap().unwrap()
    });
    let mut glass = Session::new(
        Link::new(g),
        Identity::generate(),
        None,
        SessionConfig::glass("glass"),
    );
    assert!(matches!(
        glass.handshake(PairingMode::AllowNew).await,
        Err(CoreError::PeerNotEncrypted)
    ));
    assert!(glass.bonded().is_none(), "nothing was bonded");
    let Frame::Error(e) = peer.await.unwrap() else {
        panic!("no refusal");
    };
    assert_eq!(e.code, ErrorCode::Unsupported);
}

#[tokio::test]
async fn a_stranger_with_another_key_is_refused_though_it_encrypts() {
    let (c, g) = tokio::io::duplex(1 << 16);
    let bonded_phone = Identity::generate();
    let glass_id = Identity::generate();
    let mut glass = Session::new(
        Link::new(g),
        glass_id.clone(),
        Some(bond(&bonded_phone, "phone")),
        SessionConfig::glass("glass"),
    );
    let mut stranger = Session::new(
        Link::new(c),
        Identity::generate(),
        Some(bond(&glass_id, "glass")),
        SessionConfig::companion("stranger"),
    );
    let (a, b) = tokio::join!(
        stranger.handshake(PairingMode::ReuseOnly),
        glass.handshake(PairingMode::ReuseOnly)
    );
    assert!(a.is_err());
    assert!(matches!(b, Err(CoreError::UnknownPeer)));
}

#[tokio::test]
async fn a_stranger_claiming_the_bonded_key_fails_at_the_signature() {
    let (c, g) = tokio::io::duplex(1 << 16);
    let bonded_phone = Identity::generate();
    let glass_id = Identity::generate();
    let mut glass = Session::new(
        Link::new(g),
        glass_id.clone(),
        Some(bond(&bonded_phone, "phone")),
        SessionConfig::glass("glass"),
    );
    // It says it is the bonded phone, and signs with its own key.
    let mut cfg_link = Link::new(c);
    let claimed = bonded_phone.public_b64();
    let stranger = tokio::spawn(async move {
        cfg_link
            .send(Frame::Hello(Hello {
                protocol_version: PROTOCOL_VERSION,
                role: Role::Companion,
                app_version: "x".into(),
                device_name: "phone".into(),
                identity_pub: claimed,
                capabilities: vec![capability::SESSION_CRYPTO.to_string()],
            }))
            .await
            .unwrap();
        let liar = Identity::generate();
        while let Ok(Some(frame)) = cfg_link.recv().await {
            if let Frame::AuthChallenge(ch) = frame {
                let nonce = omg_core::identity::decode_nonce(&ch.nonce).unwrap();
                cfg_link
                    .send(Frame::AuthResponse(omg_protocol::AuthResponse {
                        signature: liar.sign_challenge(&nonce),
                    }))
                    .await
                    .unwrap();
                cfg_link
                    .send(Frame::AuthChallenge(omg_protocol::AuthChallenge {
                        nonce: ch.nonce,
                    }))
                    .await
                    .unwrap();
            }
        }
    });
    assert!(matches!(
        glass.handshake(PairingMode::ReuseOnly).await,
        Err(CoreError::AuthFailed)
    ));
    drop(glass);
    stranger.await.unwrap();
}

#[tokio::test]
async fn nothing_but_the_handshake_leaves_before_the_keys_exist() {
    let (c, _g) = tokio::io::duplex(1 << 16);
    let mut session = Session::new(
        Link::new(c),
        Identity::generate(),
        None,
        SessionConfig::companion("phone"),
    );
    assert!(matches!(
        session.send(secret_frame()).await,
        Err(CoreError::KeyExchange(SecureError::NotYet(_)))
    ));
}
