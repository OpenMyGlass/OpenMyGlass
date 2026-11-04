//! The surface the phone apps use, driven the way the crate docs say an app drives it: a reader
//! thread and a pump thread on a socket the "app" owns, against the simulated Glass over TCP.

use std::{
    io::{Read, Write},
    net::TcpStream,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
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
use omg_protocol::{AttachmentKind, Locale, Settings};
use omg_transport::TcpListenerTransport;

#[derive(Default)]
struct FakeEngines {
    transcribed: AtomicU32,
    spoken: AtomicU32,
}

impl SpeechToTextEngine for FakeEngines {
    fn transcribe(
        &self,
        audio_path: String,
        _mime: String,
        _locale: NoteLocale,
    ) -> Result<String, SpeechFailure> {
        self.transcribed.fetch_add(1, Ordering::SeqCst);
        let len = std::fs::read(audio_path).unwrap().len();
        Ok(format!("{len} byte di parole"))
    }
}

impl TextToSpeechEngine for FakeEngines {
    fn synthesize(&self, text: String, _locale: NoteLocale) -> Result<SpeechAudio, SpeechFailure> {
        self.spoken.fetch_add(1, Ordering::SeqCst);
        Ok(SpeechAudio {
            sample_rate: 16_000,
            pcm: text.bytes().cycle().take(100_000).collect(),
        })
    }
}

/// What an app does around a socket.
struct App {
    core: Arc<CompanionCore>,
    events: Arc<Mutex<Vec<CoreEvent>>>,
    stop: Arc<AtomicBool>,
    socket: TcpStream,
    pump: Option<thread::JoinHandle<()>>,
    reader: Option<thread::JoinHandle<()>>,
}

impl App {
    fn connect(core: Arc<CompanionCore>, addr: &str, allow_pairing: bool) -> App {
        let socket = TcpStream::connect(addr).unwrap();
        core.link_up(link_profile_fast(), allow_pairing);
        let events = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let reader = {
            let (core, mut socket) = (core.clone(), socket.try_clone().unwrap());
            thread::spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                while let Ok(n) = socket.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    core.on_bytes(buf[..n].to_vec());
                }
            })
        };
        let pump = {
            let (core, mut socket) = (core.clone(), socket.try_clone().unwrap());
            let (events, stop) = (events.clone(), stop.clone());
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let out = core.pump();
                    if socket.write_all(&out.outgoing).is_err() {
                        break;
                    }
                    events.lock().unwrap().extend(out.events);
                    if out.close_link {
                        let _ = socket.shutdown(std::net::Shutdown::Both);
                        break;
                    }
                    thread::sleep(Duration::from_millis(2));
                }
            })
        };
        App {
            core,
            events,
            stop,
            socket,
            pump: Some(pump),
            reader: Some(reader),
        }
    }

    fn wait_for(&self, what: &str, mut done: impl FnMut(&[CoreEvent]) -> bool) {
        let start = Instant::now();
        while !done(&self.events.lock().unwrap()) {
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "timed out waiting for {what}: {:?}",
                self.events.lock().unwrap()
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn close(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
        self.pump.take().unwrap().join().unwrap();
        self.reader.take().unwrap().join().unwrap();
        self.core.link_down();
    }
}

fn core(dir: &Path, engines: &Arc<FakeEngines>) -> Arc<CompanionCore> {
    CompanionCore::new(
        dir.to_string_lossy().into_owned(),
        "test phone".into(),
        "0".into(),
        engines.clone(),
        engines.clone(),
    )
    .unwrap()
}

/// A simulated Glass in the background, serving connections one after another.
fn sim_glass(notes_dir: Option<&Path>, tts: bool) -> (String, tokio::runtime::Runtime) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt
        .block_on(TcpListenerTransport::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let mut state = SimGlassState {
        settings: Settings {
            audio_volume: Some(40),
            screen_timeout_s: Some(15),
            ..Settings::default()
        },
        ..SimGlassState::default()
    };
    state.notes = notes_dir.map(|d| {
        let mut n = SimNotes::new(d);
        n.tts_after_transcript = tts;
        n
    });
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

fn settings_of(events: &[CoreEvent]) -> Option<GlassSettings> {
    events.iter().rev().find_map(|e| match e {
        CoreEvent::SettingsChanged { settings } => Some(settings.clone()),
        _ => None,
    })
}

#[test]
fn settings_can_be_read_and_changed_and_the_bond_survives_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = Arc::new(FakeEngines::default());
    let (addr, _rt) = sim_glass(None, false);

    let phone = core(tmp.path(), &engines);
    assert_eq!(phone.snapshot().phase, LinkPhase::Down);
    let app = App::connect(phone.clone(), &addr, true);
    app.wait_for("the settings", |e| settings_of(e).is_some());
    let first = settings_of(&app.events.lock().unwrap()).unwrap();
    assert_eq!(first.audio_volume, Some(40));
    assert!(app
        .events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, CoreEvent::Paired { .. })));
    let snapshot = phone.snapshot();
    assert_eq!(snapshot.phase, LinkPhase::Running);
    assert_eq!(snapshot.bonded_to.as_deref(), Some("OpenMyGlass-Sim"));
    assert_eq!(snapshot.glass.battery_percent, Some(73));

    phone.change_settings(GlassSettings {
        audio_volume: Some(55),
        deep_sleep: Some(true),
        locale: Some(NoteLocale::It),
        ..Default::default()
    });
    app.wait_for("the change", |e| {
        settings_of(e).is_some_and(|s| s.audio_volume == Some(55))
    });
    let changed = settings_of(&app.events.lock().unwrap()).unwrap();
    assert_eq!(changed.deep_sleep, Some(true));
    assert_eq!(changed.locale, Some(NoteLocale::It));
    assert_eq!(changed.screen_timeout_s, Some(15), "untouched fields stay");
    app.close();
    assert_eq!(phone.snapshot().phase, LinkPhase::Down);

    // A new process, no permission to pair: the stored bond is enough.
    drop(phone);
    let phone = core(tmp.path(), &engines);
    assert!(phone.snapshot().bonded_to.is_some());
    let app = App::connect(phone, &addr, false);
    app.wait_for("the reconnection", |e| settings_of(e).is_some());
    app.close();
}

#[test]
fn without_a_bond_or_permission_the_link_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = Arc::new(FakeEngines::default());
    let (addr, _rt) = sim_glass(None, false);
    let app = App::connect(core(tmp.path(), &engines), &addr, false);
    app.wait_for("the refusal", |e| {
        e.iter()
            .any(|e| matches!(e, CoreEvent::LinkClosed { reason: Some(_) }))
    });
    app.close();
}

#[test]
fn notes_from_the_glass_sync_with_transcript_and_speech_on_the_phone() {
    let tmp = tempfile::tempdir().unwrap();
    let glass_dir = tmp.path().join("glass");
    let engines = Arc::new(FakeEngines::default());
    let voice = tmp.path().join("v.wav");
    let photo = tmp.path().join("p.jpg");
    let video = tmp.path().join("m.mp4");
    std::fs::write(&voice, vec![1u8; 200_000]).unwrap();
    std::fs::write(&photo, vec![2u8; 60_000]).unwrap();
    std::fs::write(&video, vec![3u8; 300_000]).unwrap();
    // Taken while the companion was away.
    let mut queue = GlassNotes::open(&glass_dir, SenderConfig::default()).unwrap();
    let source = |kind, mime: &str, path: &Path| AttachmentSource {
        kind,
        mime: mime.into(),
        path: path.to_path_buf(),
        duration_ms: None,
    };
    let (id, _) = queue
        .add_note(
            &NoteDraft {
                created_at_ms: 1,
                locale: Locale::It,
                text: Some("appunto".into()),
                attachments: vec![
                    source(AttachmentKind::VoiceRecording, "audio/wav", &voice),
                    source(AttachmentKind::Photo, "image/jpeg", &photo),
                    source(AttachmentKind::Video, "video/mp4", &video),
                ],
            },
            1,
        )
        .unwrap();
    drop(queue);

    let (addr, _rt) = sim_glass(Some(&glass_dir), true);
    let phone = core(&tmp.path().join("phone"), &engines);
    assert!(phone.notes().is_empty());
    let app = App::connect(phone.clone(), &addr, true);
    app.wait_for("the sync", |e| {
        e.iter()
            .any(|e| matches!(e, CoreEvent::NoteSynced { note_id } if *note_id == id.to_hex()))
    });
    let start = Instant::now();
    while engines.spoken.load(Ordering::SeqCst) == 0 {
        assert!(start.elapsed() < Duration::from_secs(20), "no speech");
        thread::sleep(Duration::from_millis(5));
    }
    let notes = phone.notes();
    assert_eq!(notes.len(), 1);
    let note = &notes[0];
    assert_eq!(note.origin, NoteOrigin::Glass);
    assert_eq!(note.text.as_deref(), Some("appunto"));
    assert!(note.complete);
    assert_eq!(note.transcript.as_deref(), Some("200000 byte di parole"));
    assert_eq!(note.attachments.len(), 3);
    assert_eq!(note.attachments[1].kind, NoteAttachmentKind::Photo);
    let path = note.attachments[2].path.as_ref().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), vec![3u8; 300_000]);
    app.close();
}

#[test]
fn a_note_taken_on_the_phone_is_stored_and_transcribed_with_no_link() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = Arc::new(FakeEngines::default());
    let phone = core(tmp.path(), &engines);
    let voice = tmp.path().join("v.wav");
    let photo = tmp.path().join("p.jpg");
    let video = tmp.path().join("m.mp4");
    std::fs::write(&voice, vec![1u8; 1000]).unwrap();
    std::fs::write(&photo, b"photo").unwrap();
    std::fs::write(&video, b"video").unwrap();
    let new = |kind, mime: &str, path: &Path| NewAttachment {
        kind,
        mime: mime.into(),
        path: path.to_string_lossy().into_owned(),
        duration_ms: Some(1000),
    };
    // Photos and a video together: allowed by the current default.
    phone
        .add_phone_note(
            Some("dal telefono".into()),
            NoteLocale::It,
            vec![
                new(NoteAttachmentKind::Voice, "audio/wav", &voice),
                new(NoteAttachmentKind::Photo, "image/jpeg", &photo),
                new(NoteAttachmentKind::Video, "video/mp4", &video),
            ],
        )
        .unwrap();
    let start = Instant::now();
    let note = loop {
        phone.pump();
        let note = phone.notes().remove(0);
        if note.transcript.is_some() {
            break note;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "no transcript");
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(note.origin, NoteOrigin::Phone);
    assert_eq!(note.transcript.as_deref(), Some("1000 byte di parole"));
    assert_eq!(
        std::fs::read(note.attachments[1].path.as_ref().unwrap()).unwrap(),
        b"photo"
    );

    // It is still there after a restart.
    drop(phone);
    let phone = core(tmp.path(), &engines);
    assert_eq!(phone.notes().len(), 1);

    // A note with nothing in it is refused.
    assert!(matches!(
        phone.add_phone_note(None, NoteLocale::En, Vec::new()),
        Err(CoreFailure::InvalidNote(_))
    ));
}

#[test]
fn profiles_and_names_cross_the_boundary() {
    assert_eq!(
        link_profile_moderate().max_auto_attachment_bytes,
        Some(64 * 1024 * 1024)
    );
    assert_eq!(
        link_profile_constrained().max_auto_attachment_bytes,
        Some(16 * 1024 * 1024)
    );
    assert_eq!(link_profile_fast().max_auto_attachment_bytes, None);
    assert_eq!(time_zone_names()[2], "Roma, Paris, Madrid");
}
