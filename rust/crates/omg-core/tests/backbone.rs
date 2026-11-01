//! End-to-end backbone tests over an in-memory pipe and over real TCP:
//! pairing + mutual auth + heartbeat + status + locale + notification forwarding,
//! plus rejection of a non-bonded peer.

use omg_core::{
    identity::Identity,
    session::{PairingMode, Session, SessionConfig},
    simglass::{self, SimGlassState},
    store::PairingRecord,
    CoreError,
};
use omg_protocol::{
    Frame, Locale, Notification, NotificationActionInfo, NotificationPosted, SetLocale,
    StatusRequest,
};
use omg_transport::{Link, TcpListenerTransport, TcpTransport, Transport};

fn glass_identity() -> Identity {
    Identity::generate()
}

/// Companion runs the full scripted backbone against a peer and returns what it saw.
async fn drive_companion<S: omg_transport::Stream>(
    mut session: Session<S>,
) -> Result<(u64, bool, Locale, bool), CoreError> {
    // handshake (hello + pair + auth) already done by caller; here do the app exchange.

    // Status round-trip.
    session
        .send(Frame::StatusRequest(StatusRequest::default()))
        .await?;
    // An unsolicited BatteryStatus is pushed first by sim-glass; skip to Status.
    let mut battery_percent = 0u8;
    let status = loop {
        match session.recv_app().await? {
            Some(Frame::BatteryStatus(b)) => battery_percent = b.percent,
            Some(Frame::Status(s)) => break s,
            Some(other) => panic!("unexpected {}", other.wire_name()),
            None => panic!("eof awaiting status"),
        }
    };
    assert!(battery_percent > 0, "should have received a battery push");

    // Heartbeat.
    let rtt = session.heartbeat().await?;

    // Locale push.
    session
        .send(Frame::SetLocale(SetLocale { locale: Locale::Fr }))
        .await?;
    let locale = loop {
        match session.recv_app().await? {
            Some(Frame::LocaleChanged(l)) => break l.locale,
            Some(_) => continue,
            None => panic!("eof awaiting locale"),
        }
    };

    // Notification forward (with an action, so sim-glass echoes a NotificationAction).
    session
        .send(Frame::NotificationPosted(NotificationPosted {
            notification: Notification {
                key: "msg-1".into(),
                app_name: "Signal".into(),
                title: "Giorgio".into(),
                text: "ci vediamo alle 18".into(),
                posted_at_ms: 1,
                icon_sha256: None,
                actions: vec![NotificationActionInfo {
                    index: 0,
                    title: "Reply".into(),
                    reply: true,
                }],
                wake: true,
            },
        }))
        .await?;
    let got_action = loop {
        match session.recv_app().await? {
            Some(Frame::NotificationAction(a)) => break a.key == "msg-1",
            Some(_) => continue,
            None => break false,
        }
    };

    session.close("done").await?;
    Ok((rtt, status.battery_percent.is_some(), locale, got_action))
}

#[tokio::test]
async fn full_backbone_over_duplex() {
    let (c, g) = tokio::io::duplex(64 * 1024);

    let glass = tokio::spawn(async move {
        simglass::run(
            Link::new(g),
            glass_identity(),
            None,
            SimGlassState::default(),
        )
        .await
    });

    let companion_id = Identity::generate();
    let mut session = Session::new(
        Link::new(c),
        companion_id,
        None,
        SessionConfig::companion("giorgio-pc"),
    );
    let rec = session.handshake(PairingMode::AllowNew).await.unwrap();
    assert_eq!(rec.peer_name, "OpenMyGlass-Sim");

    let (_rtt, had_battery, locale, got_action) = drive_companion(session).await.unwrap();
    assert!(had_battery);
    assert_eq!(locale, Locale::Fr);
    assert!(got_action, "sim-glass should echo a NotificationAction");

    let glass_rec = glass.await.unwrap().unwrap();
    assert_eq!(glass_rec.peer_name, "giorgio-pc");
}

#[tokio::test]
async fn full_backbone_over_tcp() {
    let listener = TcpListenerTransport::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let glass = tokio::spawn(async move {
        let link = listener.accept().await.unwrap();
        simglass::run(link, glass_identity(), None, SimGlassState::default()).await
    });

    let link = TcpTransport::new(addr).dial().await.unwrap();
    let mut session = Session::new(
        link,
        Identity::generate(),
        None,
        SessionConfig::companion("giorgio-pc"),
    );
    session.handshake(PairingMode::AllowNew).await.unwrap();
    let (_rtt, had_battery, locale, got_action) = drive_companion(session).await.unwrap();
    assert!(had_battery);
    assert_eq!(locale, Locale::Fr);
    assert!(got_action);

    glass.await.unwrap().unwrap();
}

#[tokio::test]
async fn rejects_unknown_peer() {
    // Glass is already bonded to companion A; companion B (a different identity) tries.
    let companion_a = Identity::generate();
    let bonded = PairingRecord {
        peer_pub: companion_a.public_b64(),
        peer_name: "companion-A".into(),
        bonded_at_ms: 1,
    };

    let (c, g) = tokio::io::duplex(16 * 1024);
    let glass = tokio::spawn(async move {
        simglass::run(
            Link::new(g),
            glass_identity(),
            Some(bonded),
            SimGlassState::default(),
        )
        .await
    });

    let companion_b = Identity::generate();
    let mut session = Session::new(
        Link::new(c),
        companion_b,
        None,
        SessionConfig::companion("companion-B"),
    );
    let err = session.handshake(PairingMode::AllowNew).await.unwrap_err();
    match err {
        // The companion learns it was refused via the Glass's Error frame.
        CoreError::PeerError(e) => {
            assert_eq!(e.code, omg_protocol::ErrorCode::UnknownPeer);
        }
        other => panic!("expected PeerError(UnknownPeer), got {other:?}"),
    }

    let glass_result = glass.await.unwrap();
    assert!(matches!(glass_result, Err(CoreError::UnknownPeer)));
}

#[tokio::test]
async fn reconnect_reuses_bond_and_auths() {
    // Both sides already bonded to each other: reconnect with ReuseOnly must auth.
    let companion_id = Identity::generate();
    let glass_id = glass_identity();

    let glass_bond = PairingRecord {
        peer_pub: companion_id.public_b64(),
        peer_name: "giorgio-pc".into(),
        bonded_at_ms: 1,
    };
    let companion_bond = PairingRecord {
        peer_pub: glass_id.public_b64(),
        peer_name: "OpenMyGlass-Sim".into(),
        bonded_at_ms: 1,
    };

    let (c, g) = tokio::io::duplex(16 * 1024);
    let glass = tokio::spawn(async move {
        simglass::run(
            Link::new(g),
            glass_id,
            Some(glass_bond),
            SimGlassState::default(),
        )
        .await
    });

    let mut session = Session::new(
        Link::new(c),
        companion_id,
        Some(companion_bond),
        SessionConfig::companion("giorgio-pc"),
    );
    // ReuseOnly: no new pairing, must still authenticate against the stored key.
    let rec = session.handshake(PairingMode::ReuseOnly).await.unwrap();
    assert_eq!(rec.peer_name, "OpenMyGlass-Sim");

    session.close("bye").await.unwrap();
    glass.await.unwrap().unwrap();
}
