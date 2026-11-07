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

    fn cancel(&self) {}
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
        App::connect_with(core, addr, allow_pairing, link_profile_fast())
    }

    fn connect_with(
        core: Arc<CompanionCore>,
        addr: &str,
        allow_pairing: bool,
        profile: LinkProfile,
    ) -> App {
        let socket = TcpStream::connect(addr).unwrap();
        core.link_up(profile, allow_pairing);
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
        off_head_power_off_min: Some(0),
        locale: Some(NoteLocale::It),
        ..Default::default()
    });
    app.wait_for("the change", |e| {
        settings_of(e).is_some_and(|s| s.audio_volume == Some(55))
    });
    let changed = settings_of(&app.events.lock().unwrap()).unwrap();
    assert_eq!(changed.deep_sleep, Some(true));
    assert_eq!(changed.off_head_power_off_min, Some(0), "0 means never");
    assert_eq!(changed.locale, Some(NoteLocale::It));
    assert_eq!(changed.screen_timeout_s, Some(15), "untouched fields stay");
    // Radios have their own frames, and are reported apart from the settings.
    app.wait_for("the radios", |_| {
        phone.snapshot().glass.bluetooth_on.is_some()
    });
    phone.set_radio(GlassRadio::Wifi, true);
    let start = Instant::now();
    while phone.snapshot().glass.wifi_on != Some(true) {
        assert!(start.elapsed() < Duration::from_secs(10), "no radio state");
        thread::sleep(Duration::from_millis(5));
    }
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
fn an_unpair_this_phone_cannot_store_tells_the_glass_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = Arc::new(FakeEngines::default());
    let (addr, _rt) = sim_glass(None, false);
    let phone = core(tmp.path(), &engines);
    let app = App::connect(phone.clone(), &addr, true);
    app.wait_for("the settings", |e| settings_of(e).is_some());

    // Where the store writes before its rename.
    let blocker = tmp.path().join("companion.tmp");
    std::fs::create_dir(&blocker).unwrap();
    assert!(phone.unpair().is_err());
    assert!(phone.snapshot().bonded_to.is_some());
    // The Glass would end the link on an Unpair.
    thread::sleep(Duration::from_millis(300));
    assert_eq!(phone.snapshot().phase, LinkPhase::Running);

    std::fs::remove_dir(&blocker).unwrap();
    phone.unpair().unwrap();
    assert!(phone.snapshot().bonded_to.is_none());
    // The core asks for the close itself, with no event, and the app's pump stops on it.
    let start = Instant::now();
    while !app.pump.as_ref().unwrap().is_finished() {
        assert!(start.elapsed() < Duration::from_secs(10), "the link stayed");
        thread::sleep(Duration::from_millis(5));
    }
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

/// Engines a test can switch on and hold back.
#[derive(Default)]
struct TestEngines {
    /// Absent: "no engine".
    words: Mutex<Option<String>>,
    /// While held, a transcription waits.
    hold: Mutex<()>,
    /// Transcriptions left to answer "busy".
    busy_left: AtomicU32,
    stt_now: AtomicU32,
    stt_max: AtomicU32,
    said: Mutex<Vec<String>>,
    /// Audio for each piece of speech, in bytes. 2 if unset.
    pcm_per_piece: AtomicU32,
    /// A synthesis waits until cancelled, once.
    block_tts: AtomicBool,
    tts_blocked: AtomicBool,
    cancels: AtomicU32,
    /// While set, a blocked synthesis stays blocked even once cancelled.
    hold_tts: AtomicBool,
}

impl TestEngines {
    fn with_words(words: &str) -> Arc<TestEngines> {
        let engines = Arc::new(TestEngines::default());
        *engines.words.lock().unwrap() = Some(words.into());
        engines
    }
}

impl SpeechToTextEngine for TestEngines {
    fn transcribe(&self, _: String, _: String, _: NoteLocale) -> Result<String, SpeechFailure> {
        drop(self.hold.lock().unwrap());
        let now = self.stt_now.fetch_add(1, Ordering::SeqCst) + 1;
        self.stt_max.fetch_max(now, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(5));
        self.stt_now.fetch_sub(1, Ordering::SeqCst);
        // One worker runs these, as a test elsewhere checks.
        let busy = self.busy_left.load(Ordering::SeqCst);
        if busy > 0 {
            self.busy_left.store(busy - 1, Ordering::SeqCst);
            return Err(SpeechFailure::Busy);
        }
        self.words
            .lock()
            .unwrap()
            .clone()
            .ok_or(SpeechFailure::Unavailable)
    }
}

impl TextToSpeechEngine for TestEngines {
    fn synthesize(&self, text: String, _: NoteLocale) -> Result<SpeechAudio, SpeechFailure> {
        if self.words.lock().unwrap().is_none() {
            return Err(SpeechFailure::Unavailable);
        }
        if self.block_tts.swap(false, Ordering::SeqCst) {
            self.tts_blocked.store(true, Ordering::SeqCst);
            let start = Instant::now();
            while self.cancels.load(Ordering::SeqCst) == 0 || self.hold_tts.load(Ordering::SeqCst) {
                assert!(start.elapsed() < Duration::from_secs(10), "never cancelled");
                thread::sleep(Duration::from_millis(1));
            }
            return Err(SpeechFailure::Failed {
                reason: "cancelled".into(),
            });
        }
        self.said.lock().unwrap().push(text);
        let len = match self.pcm_per_piece.load(Ordering::SeqCst) {
            0 => 2,
            n => n as usize,
        };
        Ok(SpeechAudio {
            sample_rate: 8000,
            pcm: vec![9; len],
        })
    }

    fn cancel(&self) {
        self.cancels.fetch_add(1, Ordering::SeqCst);
    }
}

fn test_core(dir: &Path, engines: &Arc<TestEngines>) -> Arc<CompanionCore> {
    CompanionCore::new(
        dir.to_string_lossy().into_owned(),
        "test phone".into(),
        "0".into(),
        engines.clone(),
        engines.clone(),
    )
    .unwrap()
}

fn add_voice_note(phone: &CompanionCore, dir: &Path, text: Option<&str>) -> String {
    let voice = dir.join("voice-source.wav");
    std::fs::write(&voice, b"audio").unwrap();
    phone
        .add_phone_note(
            text.map(Into::into),
            NoteLocale::It,
            vec![NewAttachment {
                kind: NoteAttachmentKind::Voice,
                mime: "audio/wav".into(),
                path: voice.to_string_lossy().into_owned(),
                duration_ms: None,
            }],
        )
        .unwrap()
}

/// Polls as an app with no link does, until `done`. Returns the events seen.
fn poll_until(phone: &CompanionCore, mut done: impl FnMut(&[CoreEvent]) -> bool) -> Vec<CoreEvent> {
    let start = Instant::now();
    let mut seen = Vec::new();
    loop {
        seen.extend(phone.poll());
        if done(&seen) {
            return seen;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out: {seen:?}"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn polling_alone_delivers_a_transcript_with_no_link() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = TestEngines::with_words("parole");
    let phone = test_core(tmp.path(), &engines);
    let held = engines.hold.lock().unwrap();
    add_voice_note(&phone, tmp.path(), None);
    assert!(phone.notes()[0].transcribing);
    assert!(!phone.notes()[0].speakable, "nothing to read yet");
    assert_eq!(phone.notes()[0].waiting, None);
    drop(held);
    poll_until(&phone, |_| phone.notes()[0].transcript.is_some());
    let note = phone.notes().remove(0);
    assert_eq!(note.transcript.as_deref(), Some("parole"));
    assert!(!note.transcribing);
    assert!(note.speakable);
}

#[test]
fn polling_leaves_what_the_pump_must_write_alone() {
    let (addr, _rt) = sim_glass(None, false);
    let tmp = tempfile::tempdir().unwrap();
    let engines = Arc::new(FakeEngines::default());
    let phone = core(tmp.path(), &engines);
    phone.link_up(link_profile_fast(), true);
    assert!(phone.poll().is_empty());
    assert!(!phone.pump().outgoing.is_empty(), "the handshake went out");
    phone.link_down();

    // Polling all through a real link loses no byte of it and takes no event from the pump:
    // those carry which link they came from, and must stay in order.
    let app = App::connect(phone.clone(), &addr, true);
    let start = Instant::now();
    let mut polled = Vec::new();
    while !app
        .events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, CoreEvent::SettingsChanged { .. }))
    {
        polled.extend(phone.poll());
        assert!(start.elapsed() < Duration::from_secs(10), "no settings");
    }
    for _ in 0..100 {
        polled.extend(phone.poll());
    }
    assert!(polled.is_empty(), "the poll took {polled:?}");
    let pumped = app.events.lock().unwrap().clone();
    let running = pumped
        .iter()
        .position(|e| matches!(e, CoreEvent::LinkRunning { .. }));
    let settings = pumped
        .iter()
        .position(|e| matches!(e, CoreEvent::SettingsChanged { .. }));
    assert!(running.unwrap() < settings.unwrap());
    app.close();
}

#[test]
fn speech_finished_during_a_link_reaches_the_pump_and_with_no_link_the_poll() {
    let (addr, _rt) = sim_glass(None, false);
    let tmp = tempfile::tempdir().unwrap();
    let engines = TestEngines::with_words("parole");
    let phone = test_core(tmp.path(), &engines);
    let app = App::connect(phone.clone(), &addr, true);
    app.wait_for("the link", |e| {
        e.iter().any(|e| matches!(e, CoreEvent::LinkRunning { .. }))
    });
    add_voice_note(&phone, tmp.path(), None);
    app.wait_for("the transcript", |_| phone.notes()[0].transcript.is_some());
    app.close();

    // With no link, the poll is all there is.
    let id = add_voice_note(&phone, tmp.path(), None);
    poll_until(&phone, |_| {
        phone
            .notes()
            .iter()
            .any(|n| n.id == id && n.transcript.is_some())
    });
}

#[test]
fn a_second_core_on_one_directory_is_refused_while_the_first_lives() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = TestEngines::with_words("x");
    let first = test_core(tmp.path(), &engines);
    let second = CompanionCore::new(
        tmp.path().to_string_lossy().into_owned(),
        "test phone".into(),
        "0".into(),
        engines.clone(),
        engines.clone(),
    );
    assert!(matches!(second, Err(CoreFailure::Storage(_))));
    drop(first);
    test_core(tmp.path(), &engines);
}

#[test]
fn transcriptions_run_one_at_a_time() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = TestEngines::with_words("parole");
    let phone = test_core(tmp.path(), &engines);
    for _ in 0..4 {
        add_voice_note(&phone, tmp.path(), None);
    }
    poll_until(&phone, |_| {
        phone.notes().iter().all(|n| n.transcript.is_some())
    });
    assert_eq!(engines.stt_max.load(Ordering::SeqCst), 1);
}

#[test]
fn a_busy_recognizer_is_tried_again_and_never_called_no_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = TestEngines::with_words("parole");
    engines.busy_left.store(1, Ordering::SeqCst);
    let phone = test_core(tmp.path(), &engines);
    add_voice_note(&phone, tmp.path(), None);
    poll_until(&phone, |_| phone.notes()[0].transcript.is_some());

    // Busy every time: the note says so, and asking again works.
    engines.busy_left.store(u32::MAX, Ordering::SeqCst);
    let id = add_voice_note(&phone, tmp.path(), None);
    let note = |phone: &CompanionCore| phone.notes().into_iter().find(|n| n.id == id).unwrap();
    poll_until(&phone, |_| note(&phone).transcript_error.is_some());
    assert!(note(&phone).transcript_error.unwrap().contains("busy"));
    engines.busy_left.store(0, Ordering::SeqCst);
    phone.retranscribe_note(id.clone()).unwrap();
    poll_until(&phone, |_| note(&phone).transcript.is_some());
}

#[test]
fn a_transcription_cut_short_by_the_app_stopping_runs_again_on_the_next_start() {
    let tmp = tempfile::tempdir().unwrap();
    let stuck = TestEngines::with_words("mai");
    let held = stuck.hold.lock().unwrap();
    let phone = test_core(tmp.path(), &stuck);
    add_voice_note(&phone, tmp.path(), None);
    drop(phone); // The process would be gone. Here the thread just waits on `hold`.

    let engines = TestEngines::with_words("dopo");
    let phone = test_core(tmp.path(), &engines);
    assert!(phone.notes()[0].transcribing);
    poll_until(&phone, |_| phone.notes()[0].transcript.is_some());
    assert_eq!(phone.notes()[0].transcript.as_deref(), Some("dopo"));
    drop(held);
}

#[test]
fn a_transcription_that_keeps_stopping_the_app_is_given_up_until_the_owner_asks() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = TestEngines::with_words("parole");
    let held = engines.hold.lock().unwrap();
    let phone = test_core(tmp.path(), &engines);
    let id = add_voice_note(&phone, tmp.path(), None);
    drop(phone); // The first start, ended by the engine. Nothing came back.
    for _ in 0..2 {
        let phone = test_core(tmp.path(), &engines);
        assert!(phone.notes()[0].transcribing);
        drop(phone);
    }
    let phone = test_core(tmp.path(), &engines);
    poll_until(&phone, |_| phone.notes()[0].transcript_error.is_some());
    assert!(phone.notes()[0]
        .transcript_error
        .as_ref()
        .unwrap()
        .contains("stopping the app"));

    // It stays given up on the next start, which is the point.
    drop(phone);
    let phone = test_core(tmp.path(), &engines);
    assert!(!phone.notes()[0].transcribing);

    drop(held);
    phone.retranscribe_note(id).unwrap();
    poll_until(&phone, |_| phone.notes()[0].transcript.is_some());
    assert_eq!(phone.notes()[0].transcript.as_deref(), Some("parole"));
}

#[test]
fn only_the_transcription_in_the_engine_is_charged_a_start() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = TestEngines::with_words("parole");
    let held = engines.hold.lock().unwrap();
    let phone = test_core(tmp.path(), &engines);
    add_voice_note(&phone, tmp.path(), None);
    add_voice_note(&phone, tmp.path(), None);
    let markers = || {
        std::fs::read_dir(tmp.path().join("stt-attempts"))
            .unwrap()
            .count()
    };
    let start = Instant::now();
    while markers() == 0 {
        assert!(start.elapsed() < Duration::from_secs(10), "never started");
        thread::sleep(Duration::from_millis(1));
    }
    // One worker, held in the engine: the second note cannot have been started.
    assert_eq!(markers(), 1);
    drop(held);
    poll_until(&phone, |_| {
        phone.notes().iter().all(|n| n.transcript.is_some())
    });
    assert_eq!(markers(), 0, "a result clears its count");
}

#[test]
fn no_engine_is_reported_and_asking_again_works_once_there_is_one() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = Arc::new(TestEngines::default());
    let phone = test_core(tmp.path(), &engines);
    let id = add_voice_note(&phone, tmp.path(), None);
    poll_until(&phone, |_| phone.notes()[0].transcript_error.is_some());

    // A failure is not retried behind the owner's back on every start.
    drop(phone);
    let phone = test_core(tmp.path(), &engines);
    assert!(!phone.notes()[0].transcribing);

    *engines.words.lock().unwrap() = Some("adesso".into());
    phone.retranscribe_note(id.clone()).unwrap();
    assert!(phone.notes()[0].transcribing);
    poll_until(&phone, |_| phone.notes()[0].transcript.is_some());
    let note = phone.notes().remove(0);
    assert_eq!(note.transcript.as_deref(), Some("adesso"));
    assert_eq!(note.transcript_error, None);

    for unknown in ["zz".to_string(), "ab".repeat(16)] {
        assert!(matches!(
            phone.retranscribe_note(unknown),
            Err(CoreFailure::InvalidNote(why)) if why == "no such note"
        ));
    }
}

fn ready_path(events: &[CoreEvent], wanted: u64) -> Option<String> {
    events.iter().find_map(|e| match e {
        CoreEvent::NoteSpeechReady {
            request_id, path, ..
        } if *request_id == wanted => Some(path.clone()),
        _ => None,
    })
}

#[test]
fn a_note_is_read_aloud_with_no_link_and_only_the_latest_request_counts() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = TestEngines::with_words("sentito");
    let phone = test_core(tmp.path(), &engines);
    let id = add_voice_note(&phone, tmp.path(), Some("scritto"));
    poll_until(&phone, |_| phone.notes()[0].transcript.is_some());

    phone.speak_note(id.clone(), 1).unwrap();
    let events = poll_until(&phone, |e| ready_path(e, 1).is_some());
    let first = ready_path(&events, 1).unwrap();
    assert_eq!(*engines.said.lock().unwrap(), ["scritto\nsentito"]);
    let wav = std::fs::read(&first).unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 2);
    assert_eq!(&wav[44..], [9, 9]);

    // The next request replaces the audio, and stopping removes it.
    phone.speak_note(id.clone(), 2).unwrap();
    assert!(!Path::new(&first).exists());
    let events = poll_until(&phone, |e| ready_path(e, 2).is_some());
    let second = ready_path(&events, 2).unwrap();
    assert_ne!(first, second);
    phone.stop_speaking();
    assert!(!Path::new(&second).exists());

    // No engine: the reason reaches the owner, with the request it answers.
    *engines.words.lock().unwrap() = None;
    phone.speak_note(id, 3).unwrap();
    let events = poll_until(&phone, |e| {
        e.iter()
            .any(|e| matches!(e, CoreEvent::NoteSpeechFailed { .. }))
    });
    assert!(events.iter().any(|e| matches!(
        e,
        CoreEvent::NoteSpeechFailed { request_id: 3, reason, .. }
            if reason.contains("no text-to-speech")
    )));
}

#[test]
fn stopping_cancels_the_engine_and_leaves_nothing_behind() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = TestEngines::with_words("x");
    let phone = test_core(tmp.path(), &engines);
    let id = phone
        .add_phone_note(Some("testo".into()), NoteLocale::It, Vec::new())
        .unwrap();
    engines.block_tts.store(true, Ordering::SeqCst);
    phone.speak_note(id.clone(), 1).unwrap();
    let start = Instant::now();
    while !engines.tts_blocked.load(Ordering::SeqCst) {
        assert!(start.elapsed() < Duration::from_secs(10), "never started");
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(engines.cancels.load(Ordering::SeqCst), 0);
    phone.stop_speaking();
    assert_eq!(engines.cancels.load(Ordering::SeqCst), 1);

    // Answers arrive in order, so once the next request is answered, the stopped one has been too.
    phone.speak_note(id, 2).unwrap();
    let events = poll_until(&phone, |e| ready_path(e, 2).is_some());
    assert!(ready_path(&events, 1).is_none());
    assert!(!events
        .iter()
        .any(|e| matches!(e, CoreEvent::NoteSpeechFailed { request_id: 1, .. })));
    let left: Vec<_> = std::fs::read_dir(tmp.path().join("speech"))
        .unwrap()
        .map(|e| e.unwrap().path().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, [ready_path(&events, 2).unwrap()]);
}

#[test]
fn requests_replaced_while_waiting_never_reach_the_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = TestEngines::with_words("x");
    let phone = test_core(tmp.path(), &engines);
    let id = phone
        .add_phone_note(Some("testo".into()), NoteLocale::It, Vec::new())
        .unwrap();
    engines.block_tts.store(true, Ordering::SeqCst);
    engines.hold_tts.store(true, Ordering::SeqCst);
    phone.speak_note(id.clone(), 1).unwrap();
    let start = Instant::now();
    while !engines.tts_blocked.load(Ordering::SeqCst) {
        assert!(start.elapsed() < Duration::from_secs(10), "never started");
        thread::sleep(Duration::from_millis(1));
    }
    // All of them wait behind the first, which only then ends.
    for request in 2..=20 {
        phone.speak_note(id.clone(), request).unwrap();
    }
    engines.hold_tts.store(false, Ordering::SeqCst);
    poll_until(&phone, |e| ready_path(e, 20).is_some());
    assert_eq!(engines.said.lock().unwrap().len(), 1);
}

#[test]
fn a_long_note_comes_back_as_one_audio_past_the_limit_of_the_wire() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = TestEngines::with_words("x");
    engines.pcm_per_piece.store(4 << 20, Ordering::SeqCst);
    let phone = test_core(tmp.path(), &engines);
    let long = "Una frase che si ripete. ".repeat(1000);
    let id = phone
        .add_phone_note(Some(long.clone()), NoteLocale::It, Vec::new())
        .unwrap();
    phone.speak_note(id, 1).unwrap();
    let events = poll_until(&phone, |e| ready_path(e, 1).is_some());
    let said = engines.said.lock().unwrap().clone();
    assert!(said.len() > 1);
    assert!(said.iter().all(|p| p.len() <= 3000));
    assert_eq!(said.join(" "), long.trim());
    let data = said.len() as u64 * (4 << 20);
    assert!(data > omg_protocol::MAX_TTS_AUDIO_BYTES, "too short a test");
    let wav = std::fs::metadata(ready_path(&events, 1).unwrap()).unwrap();
    assert_eq!(wav.len(), 44 + data);
}

#[test]
fn a_note_with_nothing_to_read_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = TestEngines::with_words("x");
    let phone = test_core(tmp.path(), &engines);
    let photo = tmp.path().join("p.jpg");
    std::fs::write(&photo, b"photo").unwrap();
    let id = phone
        .add_phone_note(
            None,
            NoteLocale::It,
            vec![NewAttachment {
                kind: NoteAttachmentKind::Photo,
                mime: "image/jpeg".into(),
                path: photo.to_string_lossy().into_owned(),
                duration_ms: None,
            }],
        )
        .unwrap();
    assert!(!phone.notes()[0].speakable);
    assert!(matches!(
        phone.speak_note(id, 1),
        Err(CoreFailure::InvalidNote(_))
    ));
}

#[test]
fn a_phone_note_is_deleted_with_its_files_and_stays_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = TestEngines::with_words("x");
    let phone = test_core(tmp.path(), &engines);
    let keep = phone
        .add_phone_note(Some("resta".into()), NoteLocale::It, Vec::new())
        .unwrap();
    let id = add_voice_note(&phone, tmp.path(), Some("via"));
    let path = phone
        .notes()
        .into_iter()
        .find(|n| n.id == id)
        .unwrap()
        .attachments[0]
        .path
        .clone()
        .unwrap();
    assert!(Path::new(&path).exists());

    phone.delete_note(id.clone()).unwrap();
    assert!(!Path::new(&path).exists());
    assert_eq!(phone.notes().len(), 1);
    assert!(matches!(
        phone.delete_note(id),
        Err(CoreFailure::InvalidNote(why)) if why == "no such note"
    ));
    drop(phone);
    let phone = test_core(tmp.path(), &engines);
    let notes = phone.notes();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].id, keep);
}

#[test]
fn a_note_too_big_for_the_link_says_what_it_waits_for_and_cannot_be_deleted_yet() {
    let tmp = tempfile::tempdir().unwrap();
    let glass_dir = tmp.path().join("glass");
    let photo = tmp.path().join("p.jpg");
    std::fs::write(&photo, vec![2u8; 300_000]).unwrap();
    let mut queue = GlassNotes::open(&glass_dir, SenderConfig::default()).unwrap();
    queue
        .add_note(
            &NoteDraft {
                created_at_ms: 1,
                locale: Locale::It,
                text: Some("grande".into()),
                attachments: vec![AttachmentSource {
                    kind: AttachmentKind::Photo,
                    mime: "image/jpeg".into(),
                    path: photo,
                    duration_ms: None,
                }],
            },
            1,
        )
        .unwrap();
    drop(queue);

    let (addr, _rt) = sim_glass(Some(&glass_dir), false);
    let engines = Arc::new(FakeEngines::default());
    let phone = core(&tmp.path().join("phone"), &engines);

    // Over a slow link: the first app connection pairs, with the fast profile swapped out.
    let app = App::connect_with(phone.clone(), &addr, true, link_profile_constrained());
    app.wait_for("the deferral", |e| {
        e.iter()
            .any(|e| matches!(e, CoreEvent::NoteDeferred { .. }))
    });
    let note = phone.notes().remove(0);
    assert!(!note.complete);
    assert!(!note.deletable);
    assert!(note.speakable, "its text is already here");
    assert!(note.waiting.as_deref().unwrap().contains("faster link"));
    assert!(matches!(
        phone.delete_note(note.id.clone()),
        Err(CoreFailure::InvalidNote(_))
    ));
    app.close();

    // Over a fast one it arrives, and then it can go.
    let app = App::connect(phone.clone(), &addr, false);
    app.wait_for("the sync", |e| {
        e.iter().any(|e| matches!(e, CoreEvent::NoteSynced { .. }))
    });
    let note = phone.notes().remove(0);
    assert!(note.complete);
    assert_eq!(note.waiting, None);
    assert!(note.deletable);
    phone.delete_note(note.id).unwrap();
    assert!(phone.notes().is_empty());
    app.close();

    assert!(phone.snapshot().receiving.is_none());
}

fn queue_photo(glass_dir: &Path, photo: &Path, len: usize) {
    std::fs::write(photo, vec![2u8; len]).unwrap();
    let mut queue = GlassNotes::open(glass_dir, SenderConfig::default()).unwrap();
    queue
        .add_note(
            &NoteDraft {
                created_at_ms: 1,
                locale: Locale::It,
                text: Some("foto".into()),
                attachments: vec![AttachmentSource {
                    kind: AttachmentKind::Photo,
                    mime: "image/jpeg".into(),
                    path: photo.to_path_buf(),
                    duration_ms: None,
                }],
            },
            1,
        )
        .unwrap();
}

#[test]
fn a_transfer_shows_how_far_it_has_come() {
    let tmp = tempfile::tempdir().unwrap();
    let glass_dir = tmp.path().join("glass");
    queue_photo(&glass_dir, &tmp.path().join("p.jpg"), 200_000);
    let (addr, _rt) = sim_glass(Some(&glass_dir), false);
    let engines = Arc::new(FakeEngines::default());
    let phone = core(&tmp.path().join("phone"), &engines);
    assert_eq!(phone.snapshot().receiving, None);

    let app = App::connect_with(phone.clone(), &addr, true, link_profile_constrained());
    let start = Instant::now();
    let mut seen = 0;
    let mut moved = false;
    while phone.notes().first().is_none_or(|n| !n.complete) {
        if let Some(t) = phone.snapshot().receiving {
            assert_eq!(t.total_bytes, 200_000);
            assert!(t.received_bytes >= seen, "it went backwards");
            moved |= t.received_bytes > seen && seen > 0;
            seen = t.received_bytes;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "no sync");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(moved, "the transfer was never seen moving");
    assert_eq!(phone.snapshot().receiving, None);
    app.wait_for("a report", |e| {
        e.iter().any(|e| matches!(e, CoreEvent::NoteProgress))
    });
    app.close();
}

#[cfg(unix)]
#[test]
fn a_failed_delete_ends_the_link_and_the_note_is_still_there_on_the_next() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let glass_dir = tmp.path().join("glass");
    queue_photo(&glass_dir, &tmp.path().join("p.jpg"), 1000);
    let (addr, _rt) = sim_glass(Some(&glass_dir), false);
    let engines = Arc::new(FakeEngines::default());
    let dir = tmp.path().join("phone");
    let phone = core(&dir, &engines);
    let app = App::connect(phone.clone(), &addr, true);
    app.wait_for("the sync", |e| {
        e.iter().any(|e| matches!(e, CoreEvent::NoteSynced { .. }))
    });
    let id = phone.notes().remove(0).id;

    let notes_dir = dir.join("notes-from-glass");
    let mode = |m| std::fs::Permissions::from_mode(m);

    // With no link, a failed delete has no link to end.
    app.close();
    phone.poll();
    std::fs::set_permissions(&notes_dir, mode(0o555)).unwrap();
    let failed = phone.delete_note(id.clone());
    std::fs::set_permissions(&notes_dir, mode(0o755)).unwrap();
    assert!(matches!(failed, Err(CoreFailure::Storage(_))));
    assert!(!phone
        .poll()
        .iter()
        .any(|e| matches!(e, CoreEvent::LinkClosed { .. })));
    assert!(!phone.pump().close_link);

    let app = App::connect(phone.clone(), &addr, false);
    app.wait_for("the link", |e| {
        e.iter().any(|e| matches!(e, CoreEvent::LinkRunning { .. }))
    });
    std::fs::set_permissions(&notes_dir, mode(0o555)).unwrap();
    let failed = phone.delete_note(id.clone());
    std::fs::set_permissions(&notes_dir, mode(0o755)).unwrap();
    assert!(matches!(failed, Err(CoreFailure::Storage(_))));
    app.wait_for("the link to end", |e| {
        e.iter()
            .any(|e| matches!(e, CoreEvent::LinkClosed { reason: Some(_) }))
    });
    app.close();
    assert_eq!(phone.notes()[0].id, id);

    let app = App::connect(phone.clone(), &addr, false);
    app.wait_for("the link again", |e| {
        e.iter().any(|e| matches!(e, CoreEvent::LinkRunning { .. }))
    });
    phone.delete_note(id).unwrap();
    assert!(phone.notes().is_empty());
    app.close();
}

#[test]
fn languages_map_to_the_supported_ones() {
    assert_eq!(note_locale_for_language("it".into()), NoteLocale::It);
    assert_eq!(note_locale_for_language("FR".into()), NoteLocale::Fr);
    assert_eq!(note_locale_for_language("de".into()), NoteLocale::En);
}

#[test]
fn profiles_and_names_cross_the_boundary() {
    assert_eq!(
        link_profile_moderate().max_auto_attachment_bytes,
        Some(64 * 1024 * 1024)
    );
    assert_eq!(
        link_profile_constrained().max_auto_attachment_bytes,
        Some(256 * 1024)
    );
    assert_eq!(link_profile_fast().max_auto_attachment_bytes, None);
    assert_eq!(time_zone_names()[2], "Roma, Paris, Madrid");
}
