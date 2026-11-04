//! A reusable **simulated Glass** peer.
//!
//! It plays the Glass side of the link: completes the handshake (accepting a one-time
//! pairing), then serves the backbone messages. It exists so the companion link can be
//! driven end-to-end with no hardware — in the integration tests and behind the CLI's
//! `sim-glass` command.
//!
//! This is also a precise, executable description of the **backbone glassd must
//! implement** to match the companion: see each `match` arm. With [`SimNotes`] set it also
//! keeps a note queue and syncs it, using the same state machine a real Glass would.

use std::path::PathBuf;

use omg_protocol::{
    capability, BatteryStatus, Frame, GlassStatus, Locale, LocaleChanged, NotificationAction,
    NotificationChoice, RadioState, Settings,
};
use omg_transport::{Link, Stream};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::{
    identity::Identity,
    notes::{
        link::{is_note_frame, run_link, Flow, LinkHandler},
        sender::{DeletePolicy, SenderConfig, SenderEvent},
        GlassNotes, LinkProfile,
    },
    session::{PairingMode, Session, SessionConfig},
    store::PairingRecord,
    CoreError,
};

/// Faults to inject on one connection, to exercise recovery.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SimNoteFaults {
    /// Close the link without a goodbye after sending this many attachment chunks.
    pub drop_link_after_chunks: Option<u32>,
    /// Damage this attachment chunk (1-based) after its digest was computed.
    pub corrupt_chunk: Option<u32>,
}

/// Makes the simulated Glass keep a note queue and sync it. The queue lives in `dir`, so
/// it persists across connections and restarts like on a real Glass.
#[derive(Debug, Clone)]
pub struct SimNotes {
    /// The queue's directory.
    pub dir: PathBuf,
    /// What to do with local files once verified.
    pub delete_policy: DeletePolicy,
    /// How to pull TTS audio over this link.
    pub profile: LinkProfile,
    /// Faults to inject.
    pub faults: SimNoteFaults,
    /// Ask the companion to speak each note's own text or transcript as its transcript
    /// arrives.
    pub tts_after_transcript: bool,
}

impl SimNotes {
    /// A queue in `dir` with no faults, keeping files after sync.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        SimNotes {
            dir: dir.into(),
            delete_policy: DeletePolicy::Keep,
            profile: LinkProfile::fast(),
            faults: SimNoteFaults::default(),
            tts_after_transcript: false,
        }
    }
}

/// Canned state the simulated Glass reports.
#[derive(Debug, Clone)]
pub struct SimGlassState {
    /// Device name.
    pub device_name: String,
    /// Battery percentage.
    pub battery_percent: u8,
    /// Charging flag.
    pub charging: bool,
    /// Active locale.
    pub locale: Locale,
    /// The settings a companion reads and changes.
    pub settings: Settings,
    /// glassd/ROM version.
    pub software_version: String,
    /// Keep a note queue and sync it. `None`: this Glass does not speak note sync, and
    /// does not advertise it.
    pub notes: Option<SimNotes>,
}

impl Default for SimGlassState {
    fn default() -> Self {
        SimGlassState {
            device_name: "OpenMyGlass-Sim".into(),
            battery_percent: 73,
            charging: false,
            locale: Locale::En,
            settings: Settings::default(),
            software_version: "glassd-sim/0.1.0".into(),
            notes: None,
        }
    }
}

/// Run a simulated Glass over `link`, bonded to `bonded` (or `None` to accept a new
/// pairing). Returns when the companion disconnects.
pub async fn run<S: Stream>(
    link: Link<S>,
    identity: Identity,
    bonded: Option<PairingRecord>,
    state: SimGlassState,
) -> Result<PairingRecord, CoreError> {
    let cfg = SessionConfig::glass(state.device_name.clone());
    let mut session = Session::new(link, identity, bonded, cfg);
    if state.notes.is_some() {
        session.advertise(capability::NOTES_SYNC);
    }

    session.exchange_hello().await?;
    let rec = match session.authorize(PairingMode::AllowNew).await {
        Ok(rec) => rec,
        Err(e) => {
            // authorize already sent the peer an Error frame. Keep the link open briefly
            // so the companion can read that rejection instead of seeing a reset.
            let _ = tokio::time::timeout(std::time::Duration::from_millis(200), session.recv_app())
                .await;
            return Err(e);
        }
    };
    session.authenticate().await?;
    info!(companion = %rec.peer_name, "sim-glass: companion connected");

    // Note frames go only to a companion that said it understands them: an older one
    // would treat them as a fatal decode error.
    let notes = match &state.notes {
        Some(cfg) if session.peer_supports(capability::NOTES_SYNC) => {
            let open = GlassNotes::open(
                &cfg.dir,
                SenderConfig {
                    delete_policy: cfg.delete_policy,
                    ..Default::default()
                },
            )?;
            Some((open, cfg.clone()))
        }
        _ => None,
    };

    let mut handler = SimHandler {
        state,
        notes,
        chunks_sent: 0,
    };
    let (_keep_open, no_events) = mpsc::unbounded_channel();
    // The handshake is done, so the bond stands however the link ends: the companion has
    // already stored its side, and losing ours would break every reconnect.
    if let Err(e) = run_link(session.into_link(), &mut handler, no_events).await {
        warn!(error = %e, "sim-glass: link failed");
    }

    // Our side is Glass, so from its perspective the bonded peer is the companion.
    debug!("sim-glass: companion disconnected");
    Ok(rec)
}

struct SimHandler {
    state: SimGlassState,
    notes: Option<(GlassNotes, SimNotes)>,
    chunks_sent: u32,
}

impl SimHandler {
    /// Act on what the queue reported, and apply the injected faults to what goes out.
    fn after_notes(&mut self, now: u64, out: &mut Vec<Frame>) -> Result<Flow, CoreError> {
        let Some((notes, cfg)) = &mut self.notes else {
            return Ok(Flow::Continue);
        };
        for event in notes.take_events() {
            let speak = match &event {
                SenderEvent::NoteSynced { note_id } => {
                    info!(note = %note_id.to_hex(), "sim-glass: note verified on the companion");
                    None
                }
                SenderEvent::Transcript {
                    note_id,
                    transcript,
                    ..
                } => {
                    info!(
                        note = %note_id.to_hex(),
                        text = %transcript.text,
                        error = ?transcript.error,
                        "sim-glass: transcript shown"
                    );
                    (cfg.tts_after_transcript && transcript.error.is_none())
                        .then_some((*note_id, None))
                }
                SenderEvent::TtsFinished { error, .. } => {
                    info!(error = ?error, "sim-glass: finished speaking");
                    None
                }
                SenderEvent::TtsAudio { data, .. } => {
                    debug!(bytes = data.len(), "sim-glass: playing audio");
                    None
                }
                other => {
                    warn!(event = ?other, "sim-glass: note event");
                    None
                }
            };
            if let Some((note_id, text)) = speak {
                let locale = notes
                    .machine()
                    .note(&note_id)
                    .map_or(self.state.locale, |r| r.manifest.locale);
                if let Some((_, frames)) = notes.request_tts(note_id, text, locale, now)? {
                    out.extend(frames);
                }
            }
        }
        let faults = cfg.faults;
        for frame in out.iter_mut() {
            if let Frame::NoteChunk(chunk) = frame {
                self.chunks_sent += 1;
                if faults.corrupt_chunk == Some(self.chunks_sent) {
                    warn!(offset = chunk.offset, "sim-glass: fault: damaging a chunk");
                    chunk.data[0] ^= 0xff;
                }
                if faults.drop_link_after_chunks == Some(self.chunks_sent) {
                    warn!("sim-glass: fault: dropping the link");
                    return Ok(Flow::Close);
                }
            }
        }
        Ok(Flow::Continue)
    }

    fn on_backbone(&mut self, frame: Frame, out: &mut Vec<Frame>) {
        let state = &mut self.state;
        match frame {
            Frame::StatusRequest(_) => {
                out.push(Frame::Status(GlassStatus {
                    battery_percent: Some(state.battery_percent),
                    charging: Some(state.charging),
                    storage_total_bytes: Some(16 * 1024 * 1024 * 1024),
                    storage_free_bytes: Some(9 * 1024 * 1024 * 1024),
                    device_name: Some(state.device_name.clone()),
                    software_version: Some(state.software_version.clone()),
                    screen_width_px: Some(640),
                    screen_height_px: Some(360),
                    temperature_c: Some(31.5),
                    radios: Some(RadioState {
                        wifi_on: false,
                        bt_on: true,
                        airplane_mode: false,
                        wifi_ssid: None,
                        wifi_ip: None,
                    }),
                    locale: Some(state.locale),
                }));
            }
            Frame::SettingsGet | Frame::SettingsSet(_) => {
                if let Frame::SettingsSet(change) = &frame {
                    state.settings.merge(change);
                }
                out.push(Frame::Settings(state.settings.clone()));
            }
            Frame::SetLocale(set) => {
                state.locale = set.locale;
                debug!(locale = set.locale.code(), "sim-glass: locale set");
                out.push(Frame::LocaleChanged(LocaleChanged {
                    locale: state.locale,
                }));
            }
            Frame::NotificationPosted(n) => {
                info!(
                    app = %n.notification.app_name,
                    title = %n.notification.title,
                    "sim-glass: notification shown on timeline"
                );
                // If it carries a reply action, echo that the owner dismissed it, so the
                // companion sees a NotificationAction round-trip.
                if !n.notification.actions.is_empty() {
                    out.push(Frame::NotificationAction(NotificationAction {
                        key: n.notification.key.clone(),
                        choice: NotificationChoice::Dismiss,
                    }));
                }
            }
            Frame::NotificationRemoved(r) => {
                debug!(key = %r.key, "sim-glass: notification removed");
            }
            Frame::SetRadio(_) | Frame::SetAirplaneMode(_) => {
                // SCAFFOLD: a real glassd drives rfkill/wpa_supplicant/bluez here.
                out.push(Frame::RadioState(RadioState {
                    wifi_on: false,
                    bt_on: true,
                    airplane_mode: false,
                    wifi_ssid: None,
                    wifi_ip: None,
                }));
            }
            other => {
                debug!(frame = other.wire_name(), "sim-glass: unhandled (scaffold)");
            }
        }
    }
}

impl LinkHandler for SimHandler {
    type Event = ();

    fn start(&mut self, now: u64, out: &mut Vec<Frame>) -> Result<(), CoreError> {
        // Push an initial battery status, like glassd's BatteryPowerStateReceiver.
        out.push(Frame::BatteryStatus(BatteryStatus {
            percent: self.state.battery_percent,
            charging: self.state.charging,
        }));
        if let Some((notes, cfg)) = &mut self.notes {
            out.extend(notes.link_up(cfg.profile, now)?);
        }
        self.after_notes(now, out).map(drop)
    }

    fn on_frame(
        &mut self,
        frame: Frame,
        now: u64,
        out: &mut Vec<Frame>,
    ) -> Result<Flow, CoreError> {
        if !is_note_frame(&frame) {
            self.on_backbone(frame, out);
            return Ok(Flow::Continue);
        }
        if let Some((notes, _)) = &mut self.notes {
            out.extend(notes.on_frame(frame, now)?);
        }
        self.after_notes(now, out)
    }

    fn on_tick(&mut self, now: u64, out: &mut Vec<Frame>) -> Result<Flow, CoreError> {
        if let Some((notes, _)) = &mut self.notes {
            out.extend(notes.tick(now)?);
        }
        self.after_notes(now, out)
    }

    fn on_event(&mut self, _: (), _: u64, _: &mut Vec<Frame>) -> Result<Flow, CoreError> {
        Ok(Flow::Continue)
    }

    fn finish(&mut self, _now: u64) -> Result<(), CoreError> {
        if let Some((notes, _)) = &mut self.notes {
            notes.link_down();
        }
        Ok(())
    }
}
