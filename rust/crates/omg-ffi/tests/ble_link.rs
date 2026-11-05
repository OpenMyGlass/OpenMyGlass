//! The phone's half of a BLE link, without a radio: the app's threads and the pacer as `BleLink`
//! uses them, with every run cut to GATT size each way, against the simulated Glass. A relay stands
//! in for the radio and can drop the link. The Glass's half, with the real engine and a fake
//! controller, is tested in glass-services (`crates/glassd/src/companion/ble_e2e.rs`).

use std::{
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use omg_core::{
    identity::Identity,
    notes::{AttachmentSource, GlassNotes, NoteDraft, SenderConfig},
    simglass::{self, SimGlassState, SimNotes},
};
use omg_ffi::*;
use omg_protocol::{AttachmentKind, Locale};
use omg_transport::TcpListenerTransport;

const RUN: usize = 244;

struct NoEngines;

impl SpeechToTextEngine for NoEngines {
    fn transcribe(&self, _: String, _: String, _: NoteLocale) -> Result<String, SpeechFailure> {
        Err(SpeechFailure::Unavailable)
    }
}

impl TextToSpeechEngine for NoEngines {
    fn synthesize(&self, _: String, _: NoteLocale) -> Result<SpeechAudio, SpeechFailure> {
        Err(SpeechFailure::Unavailable)
    }
}

/// Stands in for the radio. Each read hands at most one notification's worth to the core.
struct Radio {
    socket: TcpStream,
    largest_write: AtomicUsize,
    largest_read: AtomicUsize,
    received: AtomicU64,
}

/// What an app does around a `ByteLink`, with `BleLink`'s pacing.
struct App {
    core: Arc<CompanionCore>,
    events: Arc<Mutex<Vec<CoreEvent>>>,
    stop: Arc<AtomicBool>,
    radio: Arc<Radio>,
    threads: Vec<thread::JoinHandle<()>>,
}

impl App {
    fn connect(core: Arc<CompanionCore>, addr: &str, profile: LinkProfile, pairing: bool) -> App {
        let socket = TcpStream::connect(addr).unwrap();
        socket.set_nodelay(true).unwrap();
        let radio = Arc::new(Radio {
            socket,
            largest_write: AtomicUsize::new(0),
            largest_read: AtomicUsize::new(0),
            received: AtomicU64::new(0),
        });
        core.link_up(profile, pairing);
        let events = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let reader = {
            let (core, radio) = (core.clone(), radio.clone());
            thread::spawn(move || {
                let mut socket = radio.socket.try_clone().unwrap();
                let mut buf = [0u8; RUN];
                while let Ok(n) = socket.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    radio.largest_read.fetch_max(n, Ordering::SeqCst);
                    radio.received.fetch_add(n as u64, Ordering::SeqCst);
                    core.on_bytes(buf[..n].to_vec());
                }
            })
        };
        let pump = {
            let (core, radio) = (core.clone(), radio.clone());
            let (events, stop) = (events.clone(), stop.clone());
            thread::spawn(move || {
                let pacer = BleWritePacer::new();
                pacer.set_mtu(247);
                let start = Instant::now();
                let mut socket = radio.socket.try_clone().unwrap();
                while !stop.load(Ordering::SeqCst) {
                    let out = core.pump();
                    pacer.push(out.outgoing);
                    loop {
                        match pacer.next_step(start.elapsed().as_millis() as u64) {
                            BleWriteStep::Idle => break,
                            BleWriteStep::Wait { ms } => {
                                thread::sleep(Duration::from_millis(u64::from(ms)))
                            }
                            BleWriteStep::Write { bytes } => {
                                radio.largest_write.fetch_max(bytes.len(), Ordering::SeqCst);
                                if socket.write_all(&bytes).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    events.lock().unwrap().extend(out.events);
                    if out.close_link {
                        let _ = socket.shutdown(Shutdown::Both);
                        return;
                    }
                    thread::sleep(Duration::from_millis(2));
                }
            })
        };
        App {
            core,
            events,
            stop,
            radio,
            threads: vec![reader, pump],
        }
    }

    fn wait_for(&self, what: &str, mut done: impl FnMut(&[CoreEvent]) -> bool) {
        let start = Instant::now();
        while !done(&self.events.lock().unwrap()) {
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "timed out waiting for {what}: {:?}",
                self.events.lock().unwrap()
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// The link is gone, as the app sees a disconnect: everything durable stays.
    fn link_lost(self) {
        let _ = self.radio.socket.shutdown(Shutdown::Both);
        self.stop.store(true, Ordering::SeqCst);
        for t in self.threads {
            t.join().unwrap();
        }
        self.core.link_down();
    }
}

fn sim_glass(notes_dir: &Path) -> (String, tokio::runtime::Runtime) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt
        .block_on(TcpListenerTransport::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let state = SimGlassState {
        notes: Some(SimNotes::new(notes_dir)),
        ..SimGlassState::default()
    };
    rt.spawn(async move {
        let identity = Identity::generate();
        let mut bonded = None;
        while let Ok(link) = listener.accept().await {
            if let Ok(rec) =
                simglass::run(link, identity.clone(), bonded.clone(), state.clone()).await
            {
                bonded = Some(rec);
            }
        }
    });
    (addr, rt)
}

fn phone(dir: &Path) -> Arc<CompanionCore> {
    let engines = Arc::new(NoEngines);
    CompanionCore::new(
        dir.to_string_lossy().into_owned(),
        "test phone".into(),
        "0".into(),
        engines.clone(),
        engines,
    )
    .unwrap()
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31) ^ seed ^ (i >> 8) as u8)
        .collect()
}

/// Queues a note on the Glass with these attachments.
fn note_on_glass(glass: &Path, dir: &Path, files: &[(AttachmentKind, &[u8])]) -> String {
    let attachments = files
        .iter()
        .enumerate()
        .map(|(i, (kind, data))| {
            let path = dir.join(format!("source-{i}"));
            std::fs::write(&path, data).unwrap();
            AttachmentSource {
                kind: *kind,
                mime: "application/octet-stream".into(),
                path,
                duration_ms: None,
            }
        })
        .collect();
    let mut queue = GlassNotes::open(glass, SenderConfig::default()).unwrap();
    let (id, _) = queue
        .add_note(
            &NoteDraft {
                created_at_ms: 1,
                locale: Locale::It,
                text: Some("appunto".into()),
                attachments,
            },
            1,
        )
        .unwrap();
    id.to_hex()
}

fn synced(events: &[CoreEvent], id: &str) -> bool {
    events
        .iter()
        .any(|e| matches!(e, CoreEvent::NoteSynced { note_id } if note_id == id))
}

#[test]
fn a_note_syncs_with_every_run_within_the_gatt_size_and_pairing_works() {
    let tmp = tempfile::tempdir().unwrap();
    let voice = pattern(40_000, 1);
    let glass = tmp.path().join("glass");
    let id = note_on_glass(
        &glass,
        tmp.path(),
        &[(AttachmentKind::VoiceRecording, &voice)],
    );
    let (addr, _rt) = sim_glass(&glass);
    let app = App::connect(
        phone(&tmp.path().join("phone")),
        &addr,
        link_profile_constrained(),
        true,
    );
    app.wait_for("the sync", |e| synced(e, &id));
    assert!(app.radio.largest_write.load(Ordering::SeqCst) <= RUN);
    assert!(app.radio.largest_read.load(Ordering::SeqCst) <= RUN);
    let notes = app.core.notes();
    let path = notes[0].attachments[0].path.as_ref().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), voice);
    app.link_lost();
}

#[test]
fn a_link_lost_mid_transfer_comes_back_authenticated_and_resumes() {
    let tmp = tempfile::tempdir().unwrap();
    let voice = pattern(150_000, 2);
    let glass = tmp.path().join("glass");
    let id = note_on_glass(
        &glass,
        tmp.path(),
        &[(AttachmentKind::VoiceRecording, &voice)],
    );
    let (addr, _rt) = sim_glass(&glass);
    let core = phone(&tmp.path().join("phone"));
    let app = App::connect(core.clone(), &addr, link_profile_constrained(), true);
    let start = Instant::now();
    while app.radio.received.load(Ordering::SeqCst) < 50_000 {
        assert!(start.elapsed() < Duration::from_secs(60), "no transfer");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        !synced(&app.events.lock().unwrap(), &id),
        "too quick to test"
    );
    let first = app.radio.received.load(Ordering::SeqCst);
    app.link_lost();
    assert_eq!(core.snapshot().phase, LinkPhase::Down);
    assert!(core.snapshot().bonded_to.is_some());

    // The next link: no pairing allowed, so it must be the bond that lets it in.
    let app = App::connect(core.clone(), &addr, link_profile_constrained(), false);
    app.wait_for("the rest", |e| synced(e, &id));
    let again = app.radio.received.load(Ordering::SeqCst);
    assert!(
        again < 150_000 - first + 20_000,
        "started again: {again} bytes after {first}"
    );
    let notes = core.notes();
    let path = notes[0].attachments[0].path.as_ref().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), voice);
    app.link_lost();
}

#[test]
fn over_the_limit_waits_for_the_fast_profile_and_the_rest_arrives_first() {
    let tmp = tempfile::tempdir().unwrap();
    let voice = pattern(20_000, 3);
    let video = pattern(300 * 1024, 4);
    let glass = tmp.path().join("glass");
    let id = note_on_glass(
        &glass,
        tmp.path(),
        &[
            (AttachmentKind::VoiceRecording, &voice),
            (AttachmentKind::Video, &video),
        ],
    );
    let (addr, _rt) = sim_glass(&glass);
    let core = phone(&tmp.path().join("phone"));
    let app = App::connect(core.clone(), &addr, link_profile_constrained(), true);
    app.wait_for("the deferral", |e| {
        e.iter()
            .any(|e| matches!(e, CoreEvent::NoteDeferred { .. }))
    });
    assert!(!synced(&app.events.lock().unwrap(), &id));
    let notes = core.notes();
    assert!(!notes[0].complete);
    let voice_path = notes[0].attachments[0].path.as_ref().unwrap();
    assert_eq!(std::fs::read(voice_path).unwrap(), voice);
    assert_eq!(notes[0].attachments[1].path, None);
    app.link_lost();

    // TcpLink's profile, as when the owner connects over Wi-Fi.
    let app = App::connect(core.clone(), &addr, link_profile_fast(), false);
    app.wait_for("the video", |e| synced(e, &id));
    let notes = core.notes();
    let video_path = notes[0].attachments[1].path.as_ref().unwrap();
    assert_eq!(std::fs::read(video_path).unwrap(), video);
    app.link_lost();
}

#[test]
fn the_service_and_the_pacer_cross_the_boundary() {
    let s = ble_service();
    assert_eq!(s.service_uuid, "6f6d672d-626c-652d-6c69-6e6b2d763100");
    assert_eq!(s.mtu, 247);
    let pacer = BleWritePacer::new();
    pacer.set_mtu(s.mtu);
    pacer.push(vec![7; 300]);
    assert!(matches!(
        pacer.next_step(0),
        BleWriteStep::Write { bytes } if bytes.len() == 244
    ));
    assert!(matches!(
        pacer.next_step(0),
        BleWriteStep::Write { bytes } if bytes.len() == 56
    ));
    assert_eq!(pacer.next_step(0), BleWriteStep::Idle);
}
