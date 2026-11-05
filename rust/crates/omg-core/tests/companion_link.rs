//! The synchronous companion handshake against the async simulated Glass, over real TCP, with
//! a blocking socket the way a phone app owns its own. The two are separate implementations of
//! the same contract, so this is what keeps them in step.

use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

use omg_core::{
    identity::Identity,
    simglass::{self, SimGlassState},
    CompanionLink, LinkOut, PairingMode, PairingRecord,
};
use omg_protocol::{capability, Frame, FrameDecoder, Settings};
use omg_transport::TcpListenerTransport;

/// Drives the link until `until` says stop. Returns the last output batch's frames for the app.
fn pump(
    socket: &mut TcpStream,
    decoder: &mut FrameDecoder,
    link: &mut CompanionLink,
    first: Vec<LinkOut>,
    until: &mut dyn FnMut(&LinkOut) -> bool,
) {
    let mut pending = first;
    let mut buf = [0u8; 4096];
    loop {
        for out in pending.drain(..) {
            if let LinkOut::Send(frame) = &out {
                socket.write_all(&link.encode(frame).unwrap()).unwrap();
            }
            if until(&out) {
                return;
            }
        }
        while let Some(frame) = decoder.next_frame().unwrap() {
            pending.extend(link.on_frame(frame));
        }
        if pending.is_empty() {
            let n = socket.read(&mut buf).expect("the peer went quiet");
            assert!(n > 0, "the peer closed the link");
            decoder.push(&buf[..n]);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pairs_reconnects_and_reads_settings_against_the_simulated_glass() {
    let listener = TcpListenerTransport::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let glass_id = Identity::generate();
    let glass_task = {
        let glass_id = glass_id.clone();
        tokio::spawn(async move {
            let mut bonded = None;
            let state = SimGlassState {
                settings: Settings {
                    audio_volume: Some(40),
                    ..Settings::default()
                },
                ..SimGlassState::default()
            };
            for _ in 0..2 {
                let link = listener.accept().await.unwrap();
                bonded = Some(
                    simglass::run(link, glass_id.clone(), bonded.clone(), state.clone())
                        .await
                        .unwrap(),
                );
            }
        })
    };

    let phone = Identity::generate();
    let companion = tokio::task::spawn_blocking(move || {
        let connect = |bond: Option<PairingRecord>| {
            let mut socket = TcpStream::connect(addr).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut decoder = FrameDecoder::new();
            let mut link = CompanionLink::new(
                phone.clone(),
                bond,
                PairingMode::AllowNew,
                "phone",
                "test",
                vec![capability::NOTES_SYNC.to_string()],
            );
            let start = link.start();
            let mut persisted = None;
            let mut running = false;
            pump(
                &mut socket,
                &mut decoder,
                &mut link,
                start,
                &mut |out| match out {
                    LinkOut::Persist(r) => {
                        persisted = Some(r.clone());
                        false
                    }
                    LinkOut::Running { .. } => {
                        running = true;
                        true
                    }
                    LinkOut::Closed(end) => panic!("closed: {end:?}"),
                    _ => false,
                },
            );
            assert!(running);
            // The settings mirror works over this link.
            socket
                .write_all(&link.encode(&Frame::SettingsGet).unwrap())
                .unwrap();
            let mut volume = None;
            pump(
                &mut socket,
                &mut decoder,
                &mut link,
                Vec::new(),
                &mut |out| match out {
                    LinkOut::App(Frame::Settings(s)) => {
                        volume = s.audio_volume;
                        true
                    }
                    _ => false,
                },
            );
            assert_eq!(volume, Some(40));
            for out in link.close("done") {
                if let LinkOut::Send(frame) = out {
                    socket.write_all(&link.encode(&frame).unwrap()).unwrap();
                }
            }
            persisted
        };
        let bond = connect(None).expect("the first connection pairs");
        assert!(
            connect(Some(bond)).is_none(),
            "the second connection reuses the bond"
        );
    });
    tokio::time::timeout(Duration::from_secs(30), async {
        companion.await.unwrap();
        glass_task.await.unwrap();
    })
    .await
    .expect("timed out");
}
