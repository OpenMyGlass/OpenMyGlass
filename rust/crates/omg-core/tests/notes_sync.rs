//! End-to-end note sync: the real handshake, driver, store and simulated Glass, over real
//! TCP through a proxy in the middle that can damage, repeat, swallow or cut traffic.
//!
//! The link is encrypted, so a proxy on the wire can only cut it (`tests/session_crypto.rs` shows
//! that). To reach the note frames this one is given both identities and ends an encrypted
//! session on each side, which is how the faults still test the note machines.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use omg_core::{
    identity::Identity,
    notes::{
        record::{BlockReason, OutgoingState},
        run_companion,
        sender::{DeletePolicy, SenderEvent},
        AttachmentSource, CompanionNotes, CompanionRunOptions, CompanionUpdate, GlassNotes,
        LinkProfile, NoteDraft, NoteStore, OutgoingNote, ReceiverEvent, SenderConfig, SpeechError,
        SpeechToText, SynthesizeRequest, SynthesizedAudio, TextToSpeech, TranscribeRequest,
    },
    session::{PairingMode, Session, SessionConfig},
    simglass::{self, SimGlassState, SimNotes},
    store::PairingRecord,
};
use omg_protocol::{capability, AttachmentKind, Frame, Locale, NoteId, NoteTranscriptError};
use omg_transport::{Link, TcpListenerTransport, TcpTransport, Transport};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const TEST_TIMEOUT: Duration = Duration::from_secs(60);

// ------------------------------- fake engines -------------------------------

#[derive(Default)]
struct FakeSpeech {
    transcriptions: AtomicU32,
    syntheses: AtomicU32,
}

impl SpeechToText for FakeSpeech {
    fn transcribe(&self, request: &TranscribeRequest) -> Result<String, SpeechError> {
        self.transcriptions.fetch_add(1, Ordering::SeqCst);
        // Only a verified file in its final place is ever handed over.
        let audio = std::fs::read(&request.audio_path).unwrap();
        assert_eq!(request.locale, Locale::It);
        Ok(format!("trascrizione di {} byte", audio.len()))
    }
}

impl TextToSpeech for FakeSpeech {
    fn synthesize(&self, request: &SynthesizeRequest) -> Result<SynthesizedAudio, SpeechError> {
        self.syntheses.fetch_add(1, Ordering::SeqCst);
        Ok(SynthesizedAudio {
            sample_rate: 16_000,
            // Longer than one chunk, so the pull really has to loop.
            pcm: request
                .text
                .bytes()
                .cycle()
                .take(120 * 1024)
                .collect::<Vec<u8>>(),
        })
    }
}

// ------------------------------- a proxy that misbehaves ----------------------------

/// What the proxy does to one connection.
#[derive(Default, Clone, Copy)]
struct Proxy {
    /// Close both sides after this many attachment chunks went through.
    cut_after_chunks: Option<u32>,
    /// Flip a byte in this attachment chunk (1-based), keeping its digest.
    corrupt_chunk: Option<u32>,
    /// Pass every `NoteOffer` twice.
    repeat_offers: bool,
    /// Swallow acknowledgements of notes and attachments.
    swallow_acks: bool,
    /// Close both sides right after the companion's `NoteAck`. Used with `swallow_acks`: the
    /// companion has finished, the Glass has heard nothing.
    cut_after_note_ack: bool,
    /// Do not tell the Glass this companion syncs notes.
    hide_note_sync_from_glass: bool,
}

/// The Glass to reach, and what a proxy needs to stand in for both ends.
struct Upstream {
    addr: String,
    glass_id: Identity,
    companion_id: Identity,
    glass_bond: Option<PairingRecord>,
    companion_bond: Option<PairingRecord>,
}

/// What the proxy saw on one connection.
#[derive(Default)]
struct Seen {
    chunks: AtomicU32,
    chunk_bytes: AtomicU64,
    first_request_offset: Mutex<Option<u64>>,
    note_frames: AtomicU32,
}

async fn start_proxy(upstream: Upstream, proxy: Proxy) -> (String, Arc<Seen>) {
    let listener = TcpListenerTransport::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let seen = Arc::new(Seen::default());
    let seen2 = seen.clone();
    tokio::spawn(async move {
        let companion = listener.accept().await.unwrap();
        let glass = TcpTransport::new(upstream.addr).dial().await.unwrap();
        let mut facing_companion = Session::new(
            companion,
            upstream.glass_id,
            upstream.glass_bond,
            SessionConfig::glass("proxy"),
        );
        facing_companion.advertise(capability::NOTES_SYNC);
        let mut facing_glass = Session::new(
            glass,
            upstream.companion_id,
            upstream.companion_bond,
            SessionConfig::companion("proxy"),
        );
        if !proxy.hide_note_sync_from_glass {
            facing_glass.advertise(capability::NOTES_SYNC);
        }
        let (a, b) = tokio::join!(
            facing_companion.handshake(PairingMode::AllowNew),
            facing_glass.handshake(PairingMode::AllowNew),
        );
        if a.is_err() || b.is_err() {
            return;
        }
        let mut companion = facing_companion.into_link();
        let mut glass = facing_glass.into_link();
        let mut chunks = 0;
        loop {
            tokio::select! {
                frame = companion.recv() => {
                    let Ok(Some(frame)) = frame else { return };
                    if omg_core::notes::is_note_frame(&frame) {
                        seen2.note_frames.fetch_add(1, Ordering::SeqCst);
                    }
                    if let Frame::NoteChunkRequest(r) = &frame {
                        seen2.first_request_offset.lock().unwrap().get_or_insert(r.offset);
                    }
                    let is_ack = matches!(frame, Frame::NoteAttachmentAck(_) | Frame::NoteAck(_));
                    let is_note_ack = matches!(frame, Frame::NoteAck(_));
                    if !(proxy.swallow_acks && is_ack) && glass.send(frame).await.is_err() {
                        return;
                    }
                    if proxy.cut_after_note_ack && is_note_ack {
                        return;
                    }
                }
                frame = glass.recv() => {
                    let Ok(Some(mut frame)) = frame else { return };
                    if omg_core::notes::is_note_frame(&frame) {
                        seen2.note_frames.fetch_add(1, Ordering::SeqCst);
                    }
                    if let Frame::NoteChunk(chunk) = &mut frame {
                        chunks += 1;
                        seen2.chunks.fetch_add(1, Ordering::SeqCst);
                        seen2.chunk_bytes.fetch_add(chunk.data.len() as u64, Ordering::SeqCst);
                        if proxy.corrupt_chunk == Some(chunks) {
                            chunk.data[10] ^= 0x01;
                        }
                    }
                    let repeat = proxy.repeat_offers && matches!(frame, Frame::NoteOffer(_));
                    if repeat && companion.send(frame.clone()).await.is_err() {
                        return;
                    }
                    if companion.send(frame).await.is_err() {
                        return;
                    }
                    if proxy.cut_after_chunks == Some(chunks) {
                        return;
                    }
                }
            }
        }
    });
    (addr, seen)
}

/// Forwards raw bytes and cuts everything after `limit` bytes towards the companion, so the
/// link dies in the middle of a frame.
async fn start_byte_cutter(glass_addr: String, limit: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        let (companion, _) = listener.accept().await.unwrap();
        let glass = TcpStream::connect(glass_addr).await.unwrap();
        let (mut c_read, mut c_write) = companion.into_split();
        let (mut g_read, mut g_write) = glass.into_split();
        let upstream = tokio::spawn(async move {
            let _ = tokio::io::copy(&mut c_read, &mut g_write).await;
        });
        let mut buf = vec![0; 4096];
        let mut sent = 0;
        while sent < limit {
            let Ok(n) = g_read.read(&mut buf).await else {
                break;
            };
            if n == 0 {
                break;
            }
            let n = n.min(limit - sent);
            if c_write.write_all(&buf[..n]).await.is_err() {
                break;
            }
            sent += n;
        }
        upstream.abort();
    });
    addr
}

// ------------------------------- the test bed -------------------------------

/// A Glass and a companion with stable identities and their own directories, which can
/// connect any number of times, each with a fresh process's view of its disk.
struct Bed {
    _tmp: tempfile::TempDir,
    files: PathBuf,
    glass_dir: PathBuf,
    companion_dir: PathBuf,
    glass_id: Identity,
    companion_id: Identity,
    glass_bond: Option<PairingRecord>,
    companion_bond: Option<PairingRecord>,
    speech: Arc<FakeSpeech>,
}

struct RunOutcome {
    result: Result<(), omg_core::CoreError>,
    events: Vec<ReceiverEvent>,
}

impl Bed {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let files = tmp.path().join("files");
        std::fs::create_dir_all(&files).unwrap();
        Bed {
            files,
            glass_dir: tmp.path().join("glass"),
            companion_dir: tmp.path().join("companion"),
            _tmp: tmp,
            glass_id: Identity::generate(),
            companion_id: Identity::generate(),
            glass_bond: None,
            companion_bond: None,
            speech: Arc::new(FakeSpeech::default()),
        }
    }

    fn file(&self, name: &str, data: &[u8], kind: AttachmentKind) -> AttachmentSource {
        let path = self.files.join(name);
        std::fs::write(&path, data).unwrap();
        AttachmentSource {
            kind,
            mime: "application/octet-stream".into(),
            path,
            duration_ms: None,
        }
    }

    /// Seal a note with no link around, as the owner would away from the phone.
    fn take_note(&self, text: Option<&str>, attachments: Vec<AttachmentSource>) -> NoteId {
        let mut glass = GlassNotes::open(&self.glass_dir, SenderConfig::default()).unwrap();
        let (id, frames) = glass
            .add_note(
                &NoteDraft {
                    created_at_ms: 1_000,
                    locale: Locale::It,
                    text: text.map(String::from),
                    attachments,
                },
                0,
            )
            .unwrap();
        assert!(frames.is_empty(), "nothing goes out without a link");
        id
    }

    fn sim_notes(&self) -> SimNotes {
        SimNotes::new(&self.glass_dir)
    }

    /// One connection. `via` puts something between the peers: it gets the Glass and returns the one the companion should dial.
    async fn connect<F, Fut>(
        &mut self,
        sim: SimNotes,
        profile: LinkProfile,
        idle_close: bool,
        via: F,
    ) -> RunOutcome
    where
        F: FnOnce(Upstream) -> Fut,
        Fut: std::future::Future<Output = String>,
    {
        let listener = TcpListenerTransport::bind("127.0.0.1:0").await.unwrap();
        let glass_addr = listener.local_addr().unwrap().to_string();
        let id = self.glass_id.clone();
        let bond = self.glass_bond.clone();
        let glass = tokio::spawn(async move {
            let link = listener.accept().await.unwrap();
            let state = SimGlassState {
                notes: Some(sim),
                ..Default::default()
            };
            simglass::run(link, id, bond, state).await
        });

        let dial = via(Upstream {
            addr: glass_addr,
            glass_id: self.glass_id.clone(),
            companion_id: self.companion_id.clone(),
            glass_bond: self.glass_bond.clone(),
            companion_bond: self.companion_bond.clone(),
        })
        .await;
        let link = TcpTransport::new(dial).dial().await.unwrap();
        let mut session = Session::new(
            link,
            self.companion_id.clone(),
            self.companion_bond.clone(),
            SessionConfig::companion("test-companion"),
        );
        session.advertise(capability::NOTES_SYNC);
        let mode = if self.companion_bond.is_some() {
            PairingMode::ReuseOnly
        } else {
            PairingMode::AllowNew
        };
        let bond = session.handshake(mode).await.unwrap();
        self.companion_bond = Some(bond);
        assert!(session.peer_supports(capability::NOTES_SYNC));

        // Fresh from disk each connection: nothing below survives only in memory.
        let mut notes = CompanionNotes::open(&self.companion_dir, Default::default()).unwrap();
        let mut events = Vec::new();
        let options = CompanionRunOptions {
            profile,
            close_when_idle_for: idle_close.then_some(Duration::from_millis(300)),
        };
        let result = tokio::time::timeout(
            TEST_TIMEOUT,
            run_companion(
                session.into_link(),
                &mut notes,
                self.speech.clone(),
                self.speech.clone(),
                options,
                |update| {
                    if let CompanionUpdate::Note(event) = update {
                        events.push(event.clone());
                    }
                },
            ),
        )
        .await
        .expect("the run must not hang");
        let rec = tokio::time::timeout(TEST_TIMEOUT, glass)
            .await
            .expect("the Glass must not hang")
            .unwrap()
            .unwrap();
        self.glass_bond = Some(rec);
        RunOutcome { result, events }
    }

    async fn direct(&mut self, sim: SimNotes, profile: LinkProfile) -> RunOutcome {
        self.connect(sim, profile, true, |upstream| async move { upstream.addr })
            .await
    }

    fn glass_record(&self, id: &NoteId) -> Option<OutgoingNote> {
        let store = NoteStore::open(&self.glass_dir).unwrap();
        let loaded = store.load::<OutgoingNote>().unwrap();
        loaded.records.into_iter().find(|r| r.manifest.id == *id)
    }

    fn received(&self, id: &NoteId, attachment: u16) -> Option<Vec<u8>> {
        let store = NoteStore::open(&self.companion_dir).unwrap();
        std::fs::read(store.attachment_path(id, attachment)).ok()
    }

    fn partial_len(&self, id: &NoteId, attachment: u16) -> u64 {
        let store = NoteStore::open(&self.companion_dir).unwrap();
        std::fs::metadata(store.partial_path(id, attachment))
            .map(|m| m.len())
            .unwrap_or(0)
    }

    fn glass_has_files(&self, id: &NoteId) -> bool {
        let store = NoteStore::open(&self.glass_dir).unwrap();
        store.attachment_path(id, 0).exists()
    }
}

/// Bytes that differ throughout, so a misplaced chunk cannot go unnoticed.
fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u32).wrapping_mul(2_654_435_761).to_be_bytes()[0] ^ seed)
        .collect()
}

fn synced(record: &OutgoingNote) -> bool {
    matches!(record.state, OutgoingState::Synced { .. })
}

fn count(events: &[ReceiverEvent], f: impl Fn(&ReceiverEvent) -> bool) -> usize {
    events.iter().filter(|e| f(e)).count()
}

// ------------------------------- the tests -------------------------------

#[tokio::test]
async fn voice_note_with_photos_taken_offline_syncs_transcribes_and_reads_back() {
    let mut bed = Bed::new();
    let voice = pattern(70_000, 1);
    let photo_a = pattern(200_000, 2);
    let photo_b = pattern(5_000, 3);
    let attachments = vec![
        bed.file("big.jpg", &photo_a, AttachmentKind::Photo),
        bed.file("small.jpg", &photo_b, AttachmentKind::Photo),
        bed.file("voice.wav", &voice, AttachmentKind::VoiceRecording),
    ];
    let id = bed.take_note(None, attachments);

    let mut sim = bed.sim_notes();
    sim.tts_after_transcript = true;
    let run = bed.direct(sim, LinkProfile::fast()).await;
    run.result.unwrap();

    assert_eq!(bed.received(&id, 0).unwrap(), photo_a);
    assert_eq!(bed.received(&id, 1).unwrap(), photo_b);
    assert_eq!(bed.received(&id, 2).unwrap(), voice);

    // The recording came first, so the transcript does not wait for the big files.
    let verified: Vec<u16> = run
        .events
        .iter()
        .filter_map(|e| match e {
            ReceiverEvent::AttachmentVerified { attachment_id, .. } => Some(*attachment_id),
            _ => None,
        })
        .collect();
    assert_eq!(verified, vec![2, 1, 0]);

    // Speech ran on the companion, once each, and the text reached the Glass.
    assert_eq!(bed.speech.transcriptions.load(Ordering::SeqCst), 1);
    assert_eq!(bed.speech.syntheses.load(Ordering::SeqCst), 1);
    let record = bed.glass_record(&id).unwrap();
    assert!(synced(&record));
    assert_eq!(
        record.transcripts[&2].text,
        format!("trascrizione di {} byte", voice.len())
    );
    assert!(bed.glass_has_files(&id), "kept by default");

    // Nothing more happens on the next connection.
    let run = bed.direct(bed.sim_notes(), LinkProfile::fast()).await;
    run.result.unwrap();
    assert!(run.events.is_empty(), "{:?}", run.events);
    assert_eq!(bed.speech.transcriptions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn link_dropped_mid_transfer_resumes_without_refetching() {
    let mut bed = Bed::new();
    let video = pattern(2 * 1024 * 1024, 7);
    let attachments = vec![bed.file("v.mp4", &video, AttachmentKind::Video)];
    let id = bed.take_note(Some("una nota"), attachments);

    let mut sim = bed.sim_notes();
    sim.delete_policy = DeletePolicy::DeleteAfterVerified;

    // First connection: the link dies mid-transfer.
    let cut = Proxy {
        cut_after_chunks: Some(10),
        ..Default::default()
    };
    let seen_first = Arc::new(Mutex::new(None));
    let seen_slot = seen_first.clone();
    let run = bed
        .connect(sim.clone(), LinkProfile::fast(), false, |addr| async move {
            let (dial, seen) = start_proxy(addr, cut).await;
            *seen_slot.lock().unwrap() = Some(seen);
            dial
        })
        .await;
    assert_eq!(
        count(&run.events, |e| matches!(
            e,
            ReceiverEvent::NoteComplete { .. }
        )),
        0
    );
    let held = bed.partial_len(&id, 0);
    assert!(held > 0, "what arrived was kept");
    assert!(held < video.len() as u64);

    // Nothing is lost or deleted on the Glass, whatever policy says.
    assert!(bed.glass_has_files(&id));
    assert_eq!(bed.glass_record(&id).unwrap().state, OutgoingState::Queued);

    // Second connection: from fresh on-disk state on both sides, it carries on from there.
    let seen_second = Arc::new(Mutex::new(None));
    let seen_slot = seen_second.clone();
    let run = bed
        .connect(sim, LinkProfile::fast(), true, |addr| async move {
            let (dial, seen) = start_proxy(addr, Proxy::default()).await;
            *seen_slot.lock().unwrap() = Some(seen);
            dial
        })
        .await;
    run.result.unwrap();

    let seen = seen_second.lock().unwrap().clone().unwrap();
    assert_eq!(
        *seen.first_request_offset.lock().unwrap(),
        Some(held),
        "the first request resumes where the first link left off"
    );
    assert_eq!(
        seen.chunk_bytes.load(Ordering::SeqCst),
        video.len() as u64 - held,
        "only the rest travelled"
    );
    assert_eq!(bed.received(&id, 0).unwrap(), video);

    // Verified in full: now the policy applies.
    assert!(bed.glass_record(&id).is_none());
    assert!(!bed.glass_has_files(&id));
}

#[tokio::test]
async fn link_cut_in_the_middle_of_a_frame_is_a_plain_resume() {
    let mut bed = Bed::new();
    let photo = pattern(600_000, 9);
    let attachments = vec![bed.file("p.jpg", &photo, AttachmentKind::Photo)];
    let id = bed.take_note(None, attachments);

    // The limit lands inside some chunk: chunks are far larger than the handshake.
    let run = bed
        .connect(
            bed.sim_notes(),
            LinkProfile::fast(),
            false,
            |addr| async move { start_byte_cutter(addr.addr, 200_123).await },
        )
        .await;
    assert!(run.result.is_err(), "a torn frame is a link failure");
    let held = bed.partial_len(&id, 0);
    assert!(held > 0 && held < photo.len() as u64, "{held}");
    assert_eq!(held % u64::from(omg_protocol::MAX_NOTE_CHUNK_LEN), 0);
    assert_eq!(bed.glass_record(&id).unwrap().state, OutgoingState::Queued);

    let run = bed.direct(bed.sim_notes(), LinkProfile::fast()).await;
    run.result.unwrap();
    assert_eq!(bed.received(&id, 0).unwrap(), photo);
    assert!(synced(&bed.glass_record(&id).unwrap()));
}

#[tokio::test]
async fn corrupted_chunk_is_rejected_and_the_file_still_arrives_intact() {
    let mut bed = Bed::new();
    let photo = pattern(500_000, 11);
    let attachments = vec![bed.file("p.jpg", &photo, AttachmentKind::Photo)];
    let id = bed.take_note(None, attachments);

    let damage = Proxy {
        corrupt_chunk: Some(3),
        ..Default::default()
    };
    let run = bed
        .connect(
            bed.sim_notes(),
            LinkProfile::fast(),
            true,
            |addr| async move { start_proxy(addr, damage).await.0 },
        )
        .await;
    run.result.unwrap();

    let chunk_len = u64::from(omg_protocol::MAX_NOTE_CHUNK_LEN);
    let rejected: Vec<_> = run
        .events
        .iter()
        .filter_map(|e| match e {
            ReceiverEvent::ChunkRejected { offset, reason, .. } => Some((*offset, *reason)),
            _ => None,
        })
        .collect();
    assert_eq!(
        rejected,
        vec![(2 * chunk_len, omg_protocol::NoteChunkRejectReason::Corrupt)],
        "that chunk, once; the rest of its window is not blamed"
    );
    assert_eq!(bed.received(&id, 0).unwrap(), photo);
    assert!(synced(&bed.glass_record(&id).unwrap()));
}

#[tokio::test]
async fn duplicate_offers_are_ignored() {
    let mut bed = Bed::new();
    let photo = pattern(300_000, 13);
    let attachments = vec![bed.file("p.jpg", &photo, AttachmentKind::Photo)];
    let id = bed.take_note(None, attachments);

    let repeat = Proxy {
        repeat_offers: true,
        ..Default::default()
    };
    let seen_slot = Arc::new(Mutex::new(None));
    let slot = seen_slot.clone();
    let run = bed
        .connect(
            bed.sim_notes(),
            LinkProfile::fast(),
            true,
            |addr| async move {
                let (dial, seen) = start_proxy(addr, repeat).await;
                *slot.lock().unwrap() = Some(seen);
                dial
            },
        )
        .await;
    run.result.unwrap();

    let started = count(&run.events, |e| {
        matches!(e, ReceiverEvent::NoteStarted { .. })
    });
    assert_eq!(started, 1);
    let seen = seen_slot.lock().unwrap().clone().unwrap();
    assert_eq!(
        seen.chunk_bytes.load(Ordering::SeqCst),
        photo.len() as u64,
        "no byte was fetched twice"
    );
    assert_eq!(bed.received(&id, 0).unwrap(), photo);
}

#[tokio::test]
async fn lost_acknowledgement_is_recovered_by_the_repeat_offer_without_any_transfer() {
    let mut bed = Bed::new();
    let photo = pattern(100_000, 17);
    let attachments = vec![bed.file("p.jpg", &photo, AttachmentKind::Photo)];
    let id = bed.take_note(None, attachments);

    let mut sim = bed.sim_notes();
    sim.delete_policy = DeletePolicy::DeleteAfterVerified;

    // The companion finishes, but its acknowledgements never reach the Glass.
    let lose_acks = Proxy {
        swallow_acks: true,
        cut_after_note_ack: true,
        ..Default::default()
    };
    let run = bed
        .connect(sim.clone(), LinkProfile::fast(), false, |addr| async move {
            start_proxy(addr, lose_acks).await.0
        })
        .await;
    assert_eq!(
        count(&run.events, |e| matches!(
            e,
            ReceiverEvent::NoteComplete { .. }
        )),
        1
    );
    assert_eq!(bed.received(&id, 0).unwrap(), photo);
    // Without an ack the Glass has no reason to believe it: the note is all still there.
    assert_eq!(bed.glass_record(&id).unwrap().state, OutgoingState::Queued);
    assert!(bed.glass_has_files(&id));

    // Next connection: the repeat offer gets the acknowledgements again, and no bytes move.
    let slot = Arc::new(Mutex::new(None));
    let slot2 = slot.clone();
    let run = bed
        .connect(sim, LinkProfile::fast(), true, |addr| async move {
            let (dial, seen) = start_proxy(addr, Proxy::default()).await;
            *slot2.lock().unwrap() = Some(seen);
            dial
        })
        .await;
    run.result.unwrap();
    let seen = slot.lock().unwrap().clone().unwrap();
    assert_eq!(seen.chunks.load(Ordering::SeqCst), 0);
    assert!(bed.glass_record(&id).is_none(), "now it can go");
    assert_eq!(bed.received(&id, 0).unwrap(), photo);
}

#[tokio::test]
async fn a_file_that_changed_on_the_glass_blocks_its_note_and_is_never_deleted() {
    let mut bed = Bed::new();
    let photo = pattern(100_000, 47);
    let attachments = vec![bed.file("p.jpg", &photo, AttachmentKind::Photo)];
    let id = bed.take_note(None, attachments);
    let healthy_file = pattern(1_000, 53);
    let attachments = vec![bed.file("q.jpg", &healthy_file, AttachmentKind::Photo)];
    let healthy = bed.take_note(None, attachments);

    // Bit rot after sealing: every chunk passes its own check, the whole file cannot.
    let store = NoteStore::open(&bed.glass_dir).unwrap();
    let mut rotten = photo.clone();
    rotten[50_000] ^= 0x40;
    std::fs::write(store.attachment_path(&id, 0), &rotten).unwrap();

    let mut sim = bed.sim_notes();
    sim.delete_policy = DeletePolicy::DeleteAfterVerified;
    let run = bed.direct(sim, LinkProfile::fast()).await;
    run.result.unwrap();

    assert!(run.events.iter().any(|e| matches!(
        e,
        ReceiverEvent::AttachmentHashFailed { note_id, .. } if *note_id == id
    )));
    assert!(
        bed.received(&id, 0).is_none(),
        "nothing false was committed"
    );
    assert_eq!(
        bed.glass_record(&id).unwrap().state,
        OutgoingState::Blocked(BlockReason::SourceCorrupt)
    );
    assert_eq!(
        std::fs::read(store.attachment_path(&id, 0)).unwrap(),
        rotten
    );

    // It did not hold up the rest of the queue.
    assert_eq!(bed.received(&healthy, 0).unwrap(), healthy_file);
    assert!(bed.glass_record(&healthy).is_none());
}

#[tokio::test]
async fn queue_survives_a_restart_of_both_sides_in_order() {
    let mut bed = Bed::new();
    let first = bed.take_note(Some("prima"), vec![]);
    let second_file = pattern(10_000, 19);
    let attachments = vec![bed.file("p.jpg", &second_file, AttachmentKind::Photo)];
    let second = bed.take_note(Some("seconda"), attachments);

    // A restart is a fresh open from disk, which every `connect` already does; this asserts
    // the queue was still whole before any link existed.
    {
        let queue = GlassNotes::open(&bed.glass_dir, SenderConfig::default()).unwrap();
        let order: Vec<NoteId> = queue
            .machine()
            .notes()
            .iter()
            .map(|r| r.manifest.id)
            .collect();
        assert_eq!(order, vec![first, second]);
        assert_eq!(queue.machine().stats().pending_notes, 2);
    }

    let run = bed.direct(bed.sim_notes(), LinkProfile::fast()).await;
    run.result.unwrap();
    let completed: Vec<NoteId> = run
        .events
        .iter()
        .filter_map(|e| match e {
            ReceiverEvent::NoteComplete { note_id } => Some(*note_id),
            _ => None,
        })
        .collect();
    assert_eq!(completed, vec![first, second], "oldest first");
    assert_eq!(bed.received(&second, 0).unwrap(), second_file);
}

#[tokio::test]
async fn slow_link_syncs_what_fits_and_leaves_the_video_for_a_fast_one() {
    let mut bed = Bed::new();
    let voice = pattern(40_000, 23);
    let video = pattern(300_000, 29);
    let attachments = vec![
        bed.file("v.mp4", &video, AttachmentKind::Video),
        bed.file("voice.wav", &voice, AttachmentKind::VoiceRecording),
    ];
    let id = bed.take_note(None, attachments);
    let other_file = pattern(1_000, 31);
    let attachments = vec![bed.file("p.jpg", &other_file, AttachmentKind::Photo)];
    let other = bed.take_note(None, attachments);

    let slow = LinkProfile {
        max_auto_attachment_bytes: Some(100_000),
        ..LinkProfile::moderate()
    };
    let mut sim = bed.sim_notes();
    sim.delete_policy = DeletePolicy::DeleteAfterVerified;
    let run = bed.direct(sim.clone(), slow).await;
    run.result.unwrap();

    // The recording came over, and with it the transcript; the video stayed behind and
    // did not hold up the next note.
    assert_eq!(bed.received(&id, 1).unwrap(), voice);
    assert!(bed.received(&id, 0).is_none());
    assert_eq!(bed.received(&other, 0).unwrap(), other_file);
    let record = bed.glass_record(&id).unwrap();
    assert_eq!(record.state, OutgoingState::Queued);
    assert!(bed.glass_has_files(&id), "nothing deleted while incomplete");
    assert!(record.transcripts.contains_key(&1));

    let run = bed.direct(sim, LinkProfile::fast()).await;
    run.result.unwrap();
    assert_eq!(bed.received(&id, 0).unwrap(), video);
    assert!(bed.glass_record(&id).is_none());
    assert_eq!(bed.speech.transcriptions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn no_note_frame_reaches_a_companion_that_did_not_advertise_note_sync() {
    let bed = Bed::new();
    let id = bed.take_note(Some("una nota"), vec![]);

    let listener = TcpListenerTransport::bind("127.0.0.1:0").await.unwrap();
    let glass_addr = listener.local_addr().unwrap().to_string();
    let glass_id = bed.glass_id.clone();
    let sim = bed.sim_notes();
    let glass = tokio::spawn(async move {
        let link = listener.accept().await.unwrap();
        let state = SimGlassState {
            notes: Some(sim),
            ..Default::default()
        };
        simglass::run(link, glass_id, None, state).await
    });
    let (dial, seen) = start_proxy(
        Upstream {
            addr: glass_addr,
            glass_id: bed.glass_id.clone(),
            companion_id: bed.companion_id.clone(),
            glass_bond: None,
            companion_bond: None,
        },
        Proxy {
            hide_note_sync_from_glass: true,
            ..Default::default()
        },
    )
    .await;

    // An older companion: only the backbone capabilities.
    let link = TcpTransport::new(dial).dial().await.unwrap();
    let mut session = Session::new(
        link,
        bed.companion_id.clone(),
        None,
        SessionConfig::companion("old-companion"),
    );
    session.handshake(PairingMode::AllowNew).await.unwrap();
    assert!(session.peer_supports(capability::NOTES_SYNC));
    session
        .send(Frame::StatusRequest(Default::default()))
        .await
        .unwrap();
    loop {
        match session.recv_app().await.unwrap().unwrap() {
            Frame::Status(_) => break,
            Frame::BatteryStatus(_) => {}
            other => panic!("unexpected {}", other.wire_name()),
        }
    }
    session.close("done").await.unwrap();
    drop(session);
    glass.await.unwrap().unwrap();

    assert_eq!(seen.note_frames.load(Ordering::SeqCst), 0);
    assert_eq!(bed.glass_record(&id).unwrap().state, OutgoingState::Queued);
}

#[tokio::test]
async fn a_glass_without_note_sync_does_not_advertise_it() {
    let (c, g) = tokio::io::duplex(64 * 1024);
    let glass = tokio::spawn(async move {
        simglass::run(
            Link::new(g),
            Identity::generate(),
            None,
            SimGlassState::default(),
        )
        .await
    });
    let mut session = Session::new(
        Link::new(c),
        Identity::generate(),
        None,
        SessionConfig::companion("c"),
    );
    session.advertise(capability::NOTES_SYNC);
    session.handshake(PairingMode::AllowNew).await.unwrap();
    assert!(!session.peer_supports(capability::NOTES_SYNC));
    session.close("done").await.unwrap();
    drop(session);
    glass.await.unwrap().unwrap();
}

#[tokio::test]
async fn a_note_with_no_engine_syncs_all_the_same_and_reports_why_there_is_no_text() {
    use omg_core::notes::NoEngine;

    let bed = Bed::new();
    let voice = pattern(5_000, 37);
    let attachments = vec![bed.file("voice.wav", &voice, AttachmentKind::VoiceRecording)];
    let id = bed.take_note(None, attachments);

    let (c, g) = tokio::io::duplex(64 * 1024);
    let sim = bed.sim_notes();
    let glass = tokio::spawn(async move {
        let state = SimGlassState {
            notes: Some(sim),
            ..Default::default()
        };
        simglass::run(Link::new(g), Identity::generate(), None, state).await
    });
    let mut session = Session::new(
        Link::new(c),
        Identity::generate(),
        None,
        SessionConfig::companion("c"),
    );
    session.advertise(capability::NOTES_SYNC);
    session.handshake(PairingMode::AllowNew).await.unwrap();
    let mut notes = CompanionNotes::open(&bed.companion_dir, Default::default()).unwrap();
    run_companion(
        session.into_link(),
        &mut notes,
        Arc::new(NoEngine),
        Arc::new(NoEngine),
        CompanionRunOptions {
            profile: LinkProfile::fast(),
            close_when_idle_for: Some(Duration::from_millis(100)),
        },
        |_| {},
    )
    .await
    .unwrap();
    glass.await.unwrap().unwrap();

    let record = bed.glass_record(&id).unwrap();
    assert!(synced(&record));
    assert_eq!(
        record.transcripts[&0].error,
        Some(NoteTranscriptError::EngineUnavailable)
    );
}

/// Both directions busy at once over a pipe with almost no buffer. A peer that only read
/// between its writes would deadlock here. Also shows the core does not need TCP: this is
/// any byte stream, and the chunk sizes are those for a small link.
#[tokio::test]
async fn tiny_buffers_and_traffic_both_ways_do_not_deadlock() {
    let bed = Bed::new();
    let voice = pattern(30_000, 41);
    let photo = pattern(150_000, 43);
    let attachments = vec![
        bed.file("voice.wav", &voice, AttachmentKind::VoiceRecording),
        bed.file("p.jpg", &photo, AttachmentKind::Photo),
    ];
    let id = bed.take_note(None, attachments);

    let (c, g) = tokio::io::duplex(128);
    let mut sim = bed.sim_notes();
    sim.profile = LinkProfile::constrained();
    sim.tts_after_transcript = true;
    let glass = tokio::spawn(async move {
        let state = SimGlassState {
            notes: Some(sim),
            ..Default::default()
        };
        simglass::run(Link::new(g), Identity::generate(), None, state).await
    });
    let mut session = Session::new(
        Link::new(c),
        Identity::generate(),
        None,
        SessionConfig::companion("c"),
    );
    session.advertise(capability::NOTES_SYNC);
    session.handshake(PairingMode::AllowNew).await.unwrap();
    let mut notes = CompanionNotes::open(&bed.companion_dir, Default::default()).unwrap();
    tokio::time::timeout(
        TEST_TIMEOUT,
        run_companion(
            session.into_link(),
            &mut notes,
            bed.speech.clone(),
            bed.speech.clone(),
            CompanionRunOptions {
                profile: LinkProfile::constrained(),
                close_when_idle_for: Some(Duration::from_millis(100)),
            },
            |_| {},
        ),
    )
    .await
    .expect("must not deadlock")
    .unwrap();
    glass.await.unwrap().unwrap();

    assert_eq!(bed.received(&id, 0).unwrap(), voice);
    assert_eq!(bed.received(&id, 1).unwrap(), photo);
    assert_eq!(bed.speech.syntheses.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_glass_only_plays_speech_it_could_verify() {
    // The driver on its own, with the Glass-side events visible: a full TTS round.
    let tmp = tempfile::tempdir().unwrap();
    let dir: &Path = tmp.path();
    let mut glass = GlassNotes::open(dir.join("g"), SenderConfig::default()).unwrap();
    let mut companion = CompanionNotes::open(dir.join("c"), Default::default()).unwrap();
    let engine = FakeSpeech::default();
    let id = NoteId([5; 16]);

    assert!(
        glass
            .request_tts(id, Some("ciao".into()), Locale::It, 0)
            .unwrap()
            .is_none(),
        "no link, no speech: there is no engine on the Glass to fall back on"
    );

    glass.link_up(LinkProfile::moderate(), 0).unwrap();
    companion.link_up(LinkProfile::moderate(), 0).unwrap();
    let (_, mut to_companion) = glass
        .request_tts(id, Some("ciao".into()), Locale::It, 0)
        .unwrap()
        .unwrap();
    let mut to_glass: Vec<Frame> = Vec::new();
    for _ in 0..1_000 {
        for frame in to_companion.drain(..) {
            to_glass.extend(companion.on_frame(frame, 1).unwrap());
        }
        for job in companion.take_speech_jobs() {
            let outcome = job.run(&engine, &engine);
            to_glass.extend(companion.complete_speech(outcome, 1).unwrap());
        }
        for frame in to_glass.drain(..) {
            to_companion.extend(glass.on_frame(frame, 1).unwrap());
        }
        if to_companion.is_empty() {
            break;
        }
    }
    let played: usize = glass
        .take_events()
        .iter()
        .map(|e| match e {
            SenderEvent::TtsAudio { data, .. } => data.len(),
            SenderEvent::TtsFinished { error, .. } => {
                assert_eq!(*error, None);
                0
            }
            other => panic!("{other:?}"),
        })
        .sum();
    assert_eq!(played, 120 * 1024);
    assert!(companion.machine().peer_status().is_none());
}
