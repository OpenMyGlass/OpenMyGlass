//! The companion core for the phone apps, through UniFFI: one Rust surface, Kotlin and Swift
//! bindings generated from it. See `docs/companion-mobile.md`.
//!
//! **The app owns the socket.** Nothing here opens one or names a radio, because iOS has no
//! Bluetooth Classic for this use: an Android app may hand in an RFCOMM `BluetoothSocket`, an
//! iOS app a BLE or `Network.framework` stream, and both call the same functions. The one
//! thing a link says about itself is a [`LinkProfile`]: how much to pull at a time.
//!
//! Using it takes two threads per link:
//!
//! ```text
//! reader:  loop { n = socket.read(buf); core.on_bytes(buf[..n]); wake(pump) }
//! pump:    loop { wait(wake, at most 250 ms); o = core.pump();
//!                 socket.write(o.outgoing); show(o.events); if o.close_link { break } }
//! ```
//!
//! Bytes leave in the order the core produced them whichever thread called it, and a blocked
//! write never stops reading, which matters on small Bluetooth buffers where two peers could
//! otherwise each wait for the other to read.
//!
//! Every call is safe from any thread. Only [`CompanionCore::add_phone_note`] and the speech
//! engines can take long, and neither holds the core's lock while it runs.

uniffi::setup_scaffolding!();

use std::{
    path::PathBuf,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
};

use omg_core::{
    notes::{
        record::OutgoingState, AttachmentSource, CompanionNotes, GlassNotes, IncomingNote,
        NoteDraft, OutgoingNote, ReceiverEvent, RetryPolicy, SenderConfig, SpeechError, SpeechJob,
        SpeechOutcome, SpeechToText, SynthesizeRequest, SynthesizedAudio, TextToSpeech,
        TranscribeRequest,
    },
    now_ms, CompanionLink, LinkEnd, LinkOut, PairingMode, Store,
};
use omg_protocol::{
    capability, AttachmentKind, BatteryStatus, Frame, FrameDecoder, GlassStatus, NoteDeferReason,
    NoteId, NoteRejectReason, NoteSyncStatus, NoteTranscript, NoteTranscriptError, Radio,
    RadioState, SetAirplaneMode, SetLocale, SetRadio, Settings, StatusRequest,
};

const STATE_FILE: &str = "companion.json";
const GLASS_NOTES_DIR: &str = "notes-from-glass";
const PHONE_NOTES_DIR: &str = "notes-from-phone";

// ------------------------------ errors ------------------------------

/// Why a call failed.
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum CoreFailure {
    /// The state on disk could not be read or written.
    #[error("storage: {0}")]
    Storage(String),
    /// A note breaks the limits, or a file of it could not be read.
    #[error("invalid note: {0}")]
    InvalidNote(String),
}

/// Why a speech engine produced nothing.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SpeechFailure {
    /// No engine here, or none for this language.
    #[error("no speech engine available")]
    Unavailable,
    /// The engine failed.
    #[error("speech engine failed: {reason}")]
    Failed {
        /// Why.
        reason: String,
    },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for SpeechFailure {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        SpeechFailure::Failed { reason: e.reason }
    }
}

impl From<SpeechFailure> for SpeechError {
    fn from(e: SpeechFailure) -> Self {
        match e {
            SpeechFailure::Unavailable => SpeechError::Unavailable,
            SpeechFailure::Failed { reason } => SpeechError::Failed(reason),
        }
    }
}

// ------------------------------ plain data ------------------------------

/// How to pull over a link. The app picks one for the link it opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct LinkProfile {
    /// Bytes per chunk.
    pub chunk_len: u32,
    /// Chunks outstanding at once.
    pub window: u32,
    /// Bigger attachments wait for a faster link. `None`: no limit.
    pub max_auto_attachment_bytes: Option<u64>,
}

impl From<omg_core::notes::LinkProfile> for LinkProfile {
    fn from(p: omg_core::notes::LinkProfile) -> Self {
        LinkProfile {
            chunk_len: p.chunk_len,
            window: p.window,
            max_auto_attachment_bytes: p.max_auto_attachment_bytes,
        }
    }
}

impl From<LinkProfile> for omg_core::notes::LinkProfile {
    fn from(p: LinkProfile) -> Self {
        omg_core::notes::LinkProfile {
            chunk_len: p.chunk_len,
            window: p.window,
            max_auto_attachment_bytes: p.max_auto_attachment_bytes,
        }
    }
}

/// For a low-throughput link, such as BLE.
#[uniffi::export]
pub fn link_profile_constrained() -> LinkProfile {
    omg_core::notes::LinkProfile::constrained().into()
}

/// For a moderate link, such as Bluetooth Classic.
#[uniffi::export]
pub fn link_profile_moderate() -> LinkProfile {
    omg_core::notes::LinkProfile::moderate().into()
}

/// For a fast link, such as Wi-Fi.
#[uniffi::export]
pub fn link_profile_fast() -> LinkProfile {
    omg_core::notes::LinkProfile::fast().into()
}

/// The GATT service a BLE link uses. See `omg_core::ble`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BleService {
    pub service_uuid: String,
    /// Phone to Glass: write without response.
    pub rx_uuid: String,
    /// Glass to phone: notify, with the descriptor below.
    pub tx_uuid: String,
    pub cccd_uuid: String,
    /// The ATT MTU to ask for.
    pub mtu: u16,
}

#[uniffi::export]
pub fn ble_service() -> BleService {
    use omg_core::ble;
    BleService {
        service_uuid: ble::SERVICE_UUID.into(),
        rx_uuid: ble::RX_UUID.into(),
        tx_uuid: ble::TX_UUID.into(),
        cccd_uuid: ble::CCCD_UUID.into(),
        mtu: ble::REQUESTED_MTU,
    }
}

/// What to do next with the writes of a BLE link.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum BleWriteStep {
    /// One write without response. Ask again when the stack has taken it.
    Write { bytes: Vec<u8> },
    /// Nothing may go for this long.
    Wait { ms: u32 },
    /// Everything was handed over.
    Idle,
}

/// Spaces the writes of a BLE link, because a phone's stack would otherwise queue them for
/// seconds ahead of anything sent later. One per link. See `omg_core::ble`.
#[derive(uniffi::Object)]
pub struct BleWritePacer {
    inner: Mutex<omg_core::ble::WritePacer>,
}

impl Default for BleWritePacer {
    fn default() -> Self {
        BleWritePacer {
            inner: Mutex::new(omg_core::ble::WritePacer::new()),
        }
    }
}

#[uniffi::export]
impl BleWritePacer {
    #[uniffi::constructor]
    pub fn new() -> Arc<BleWritePacer> {
        Arc::new(BleWritePacer::default())
    }

    /// The negotiated ATT MTU.
    pub fn set_mtu(&self, mtu: u16) {
        self.lock().set_mtu(mtu);
    }

    /// Bytes to send, as `CoreOutput::outgoing` gives them.
    pub fn push(&self, bytes: Vec<u8>) {
        self.lock().push(&bytes);
    }

    /// What to do at `now_ms`, on a clock that does not go backwards.
    pub fn next_step(&self, now_ms: u64) -> BleWriteStep {
        match self.lock().next_step(now_ms) {
            omg_core::ble::WriteStep::Write(bytes) => BleWriteStep::Write { bytes },
            omg_core::ble::WriteStep::Wait(ms) => BleWriteStep::Wait { ms },
            omg_core::ble::WriteStep::Idle => BleWriteStep::Idle,
        }
    }
}

impl BleWritePacer {
    fn lock(&self) -> std::sync::MutexGuard<'_, omg_core::ble::WritePacer> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// Names of the time zones `GlassSettings::timezone_id` indexes.
#[uniffi::export]
pub fn time_zone_names() -> Vec<String> {
    omg_protocol::TIME_ZONE_NAMES
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// A UI language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum NoteLocale {
    /// Italiano.
    It,
    /// English.
    En,
    /// Français.
    Fr,
    /// Español.
    Es,
}

impl From<omg_protocol::Locale> for NoteLocale {
    fn from(l: omg_protocol::Locale) -> Self {
        match l {
            omg_protocol::Locale::It => NoteLocale::It,
            omg_protocol::Locale::En => NoteLocale::En,
            omg_protocol::Locale::Fr => NoteLocale::Fr,
            omg_protocol::Locale::Es => NoteLocale::Es,
        }
    }
}

impl From<NoteLocale> for omg_protocol::Locale {
    fn from(l: NoteLocale) -> Self {
        match l {
            NoteLocale::It => omg_protocol::Locale::It,
            NoteLocale::En => omg_protocol::Locale::En,
            NoteLocale::Fr => omg_protocol::Locale::Fr,
            NoteLocale::Es => omg_protocol::Locale::Es,
        }
    }
}

/// The settings on the Glass. In a change, only the fields that are set are touched. The Glass
/// clamps values to its ranges and answers with the full result, which arrives as
/// [`CoreEvent::SettingsChanged`]. A field the Glass does not report stays `None`: hide it.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct GlassSettings {
    /// Prism brightness, 0-100.
    #[uniffi(default)]
    pub brightness: Option<u32>,
    /// Wake on looking up.
    #[uniffi(default)]
    pub head_wake: Option<bool>,
    /// The look-up angle, degrees.
    #[uniffi(default)]
    pub head_wake_angle_deg: Option<u32>,
    /// On-head detection.
    #[uniffi(default)]
    pub on_head_detection: Option<bool>,
    /// Earpiece volume, 0-100.
    #[uniffi(default)]
    pub audio_volume: Option<u32>,
    /// 24-hour clock.
    #[uniffi(default)]
    pub time_24h: Option<bool>,
    /// Index into [`time_zone_names`].
    #[uniffi(default)]
    pub timezone_id: Option<u32>,
    /// Energy policy.
    #[uniffi(default)]
    pub power_saver: Option<bool>,
    /// Suspend to RAM when idle.
    #[uniffi(default)]
    pub deep_sleep: Option<bool>,
    /// Idle before suspending, seconds.
    #[uniffi(default)]
    pub deep_sleep_idle_s: Option<u32>,
    /// Minutes off the head before powering off: 0 = never, at most 720.
    #[uniffi(default)]
    pub off_head_power_off_min: Option<u32>,
    /// Wink to capture.
    #[uniffi(default)]
    pub wink: Option<bool>,
    /// Screen timeout, seconds.
    #[uniffi(default)]
    pub screen_timeout_s: Option<u32>,
    /// Head nudge to wake.
    #[uniffi(default)]
    pub nudge: Option<bool>,
    /// UI language.
    #[uniffi(default)]
    pub locale: Option<NoteLocale>,
}

impl From<&Settings> for GlassSettings {
    fn from(s: &Settings) -> Self {
        GlassSettings {
            brightness: s.brightness.map(u32::from),
            head_wake: s.head_wake,
            head_wake_angle_deg: s.head_wake_angle_deg.map(u32::from),
            on_head_detection: s.on_head_detection,
            audio_volume: s.audio_volume.map(u32::from),
            time_24h: s.time_24h,
            timezone_id: s.timezone_id,
            power_saver: s.power_saver,
            deep_sleep: s.deep_sleep,
            deep_sleep_idle_s: s.deep_sleep_idle_s,
            off_head_power_off_min: s.off_head_power_off_min,
            wink: s.wink,
            screen_timeout_s: s.screen_timeout_s,
            nudge: s.nudge,
            locale: s.locale.map(Into::into),
        }
    }
}

fn saturate_u8(v: u32) -> u8 {
    v.min(u32::from(u8::MAX)) as u8
}

impl From<&GlassSettings> for Settings {
    fn from(s: &GlassSettings) -> Self {
        Settings {
            brightness: s.brightness.map(saturate_u8),
            head_wake: s.head_wake,
            head_wake_angle_deg: s.head_wake_angle_deg.map(saturate_u8),
            on_head_detection: s.on_head_detection,
            audio_volume: s.audio_volume.map(saturate_u8),
            time_24h: s.time_24h,
            timezone_id: s.timezone_id,
            power_saver: s.power_saver,
            deep_sleep: s.deep_sleep,
            deep_sleep_idle_s: s.deep_sleep_idle_s,
            off_head_power_off_min: s.off_head_power_off_min,
            wink: s.wink,
            screen_timeout_s: s.screen_timeout_s,
            nudge: s.nudge,
            locale: s.locale.map(Into::into),
            radios: None,
        }
    }
}

/// A radio that can be switched from here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum GlassRadio {
    /// Wi-Fi.
    Wifi,
    /// Bluetooth.
    Bluetooth,
}

/// What the Glass last said about itself. `None` until it does.
#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct GlassState {
    /// Its name.
    pub name: Option<String>,
    /// Battery, 0-100.
    pub battery_percent: Option<u32>,
    /// Charging.
    pub charging: Option<bool>,
    /// Its software.
    pub software_version: Option<String>,
    /// Internal storage free, bytes.
    pub storage_free_bytes: Option<u64>,
    /// Wi-Fi on.
    pub wifi_on: Option<bool>,
    /// Bluetooth on.
    pub bluetooth_on: Option<bool>,
    /// Airplane mode on.
    pub airplane_mode: Option<bool>,
    /// The settings.
    pub settings: Option<GlassSettings>,
    /// Notes still waiting on the Glass.
    pub pending_notes: Option<u32>,
    /// Bytes of them still waiting.
    pub pending_bytes: Option<u64>,
}

/// Where the link stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LinkPhase {
    /// No link.
    Down,
    /// The socket is open and the handshake running.
    Handshake,
    /// The Glass is authenticated.
    Running,
}

/// A snapshot for a UI.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CoreSnapshot {
    /// The link.
    pub phase: LinkPhase,
    /// The bonded Glass's name, if bonded.
    pub bonded_to: Option<String>,
    /// Our public key, base64.
    pub identity: String,
    /// What the Glass last said.
    pub glass: GlassState,
}

/// Something that happened since the last [`CompanionCore::pump`].
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum CoreEvent {
    /// A new bond was stored.
    Paired {
        /// The Glass's name.
        glass_name: String,
    },
    /// The Glass is authenticated.
    LinkRunning {
        /// Its name.
        glass_name: String,
        /// Whether it syncs notes.
        notes_sync: bool,
    },
    /// The link ended. Close the socket.
    LinkClosed {
        /// Why, if it was not a clean end.
        reason: Option<String>,
    },
    /// The Glass's settings arrived or changed.
    SettingsChanged {
        /// The whole mirror.
        settings: GlassSettings,
    },
    /// Battery, status or radios changed: read [`CompanionCore::snapshot`].
    GlassStateChanged,
    /// The note list changed: read [`CompanionCore::notes`].
    NotesChanged,
    /// A note has been stored and verified here.
    NoteSynced {
        /// The note.
        note_id: String,
    },
    /// The Glass was told to wait with this note.
    NoteDeferred {
        /// The note.
        note_id: String,
        /// Why, for the owner.
        reason: String,
    },
    /// This side refused a note. It stays on the Glass.
    NoteRefused {
        /// The note.
        note_id: String,
        /// Why, for the owner.
        reason: String,
    },
}

/// What [`CompanionCore::pump`] hands over.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CoreOutput {
    /// Write these bytes to the socket, in this order, before the next pump.
    pub outgoing: Vec<u8>,
    /// What happened.
    pub events: Vec<CoreEvent>,
    /// Write what is left and close the socket, then call [`CompanionCore::link_down`].
    pub close_link: bool,
}

/// What an attachment is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum NoteAttachmentKind {
    /// The audio of a voice note.
    Voice,
    /// A photo.
    Photo,
    /// A video.
    Video,
}

impl From<AttachmentKind> for NoteAttachmentKind {
    fn from(k: AttachmentKind) -> Self {
        match k {
            AttachmentKind::VoiceRecording => NoteAttachmentKind::Voice,
            AttachmentKind::Photo => NoteAttachmentKind::Photo,
            AttachmentKind::Video => NoteAttachmentKind::Video,
        }
    }
}

impl From<NoteAttachmentKind> for AttachmentKind {
    fn from(k: NoteAttachmentKind) -> Self {
        match k {
            NoteAttachmentKind::Voice => AttachmentKind::VoiceRecording,
            NoteAttachmentKind::Photo => AttachmentKind::Photo,
            NoteAttachmentKind::Video => AttachmentKind::Video,
        }
    }
}

/// One attachment of a note.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NoteAttachmentInfo {
    /// What it is.
    pub kind: NoteAttachmentKind,
    /// MIME type.
    pub mime: String,
    /// Full size, bytes.
    pub size: u64,
    /// Length of audio or video, when known.
    pub duration_ms: Option<u32>,
    /// Where to read it, once it is complete here. Read only.
    pub path: Option<String>,
}

/// Where a note was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum NoteOrigin {
    /// On the Glass, and synced here.
    Glass,
    /// On this phone.
    Phone,
}

/// A note, for a list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NoteInfo {
    /// Its id.
    pub id: String,
    /// Where it was taken.
    pub origin: NoteOrigin,
    /// When, by the clock of the device that took it, Unix ms.
    pub created_at_ms: u64,
    /// Its language.
    pub locale: NoteLocale,
    /// Its text.
    pub text: Option<String>,
    /// The transcript of its recording.
    pub transcript: Option<String>,
    /// Why it has no transcript, once speech-to-text has run.
    pub transcript_error: Option<String>,
    /// Its attachments.
    pub attachments: Vec<NoteAttachmentInfo>,
    /// Every attachment is here and verified.
    pub complete: bool,
}

/// One file of a new note on this phone. It is copied in; the original is left alone.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NewAttachment {
    /// What it is.
    pub kind: NoteAttachmentKind,
    /// MIME type.
    pub mime: String,
    /// Where it is now.
    pub path: String,
    /// Length of audio or video, when known.
    pub duration_ms: Option<u32>,
}

// ------------------------------ speech engines ------------------------------

/// A speech-to-text engine, implemented by the app. Blocking: it is called on its own thread,
/// never on one servicing the link.
#[uniffi::export(with_foreign)]
pub trait SpeechToTextEngine: Send + Sync {
    /// Transcribe the recording at `audio_path`. Read it; do not move or change it.
    fn transcribe(
        &self,
        audio_path: String,
        mime: String,
        locale: NoteLocale,
    ) -> Result<String, SpeechFailure>;
}

/// Speech from a [`TextToSpeechEngine`]: PCM 16-bit little-endian mono.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SpeechAudio {
    /// Sample rate.
    pub sample_rate: u32,
    /// The samples.
    pub pcm: Vec<u8>,
}

/// A text-to-speech engine, implemented by the app. Speech always runs on the companion, never
/// on the Glass. Blocking, as above.
#[uniffi::export(with_foreign)]
pub trait TextToSpeechEngine: Send + Sync {
    /// Speak `text`.
    fn synthesize(&self, text: String, locale: NoteLocale) -> Result<SpeechAudio, SpeechFailure>;
}

struct Engines {
    stt: Arc<dyn SpeechToTextEngine>,
    tts: Arc<dyn TextToSpeechEngine>,
}

impl SpeechToText for Engines {
    fn transcribe(&self, request: &TranscribeRequest) -> Result<String, SpeechError> {
        self.stt
            .transcribe(
                request.audio_path.to_string_lossy().into_owned(),
                request.mime.clone(),
                request.locale.into(),
            )
            .map_err(Into::into)
    }
}

impl TextToSpeech for Engines {
    fn synthesize(&self, request: &SynthesizeRequest) -> Result<SynthesizedAudio, SpeechError> {
        self.tts
            .synthesize(request.text.clone(), request.locale.into())
            .map(|a| SynthesizedAudio {
                sample_rate: a.sample_rate,
                pcm: a.pcm,
            })
            .map_err(Into::into)
    }
}

// ------------------------------ the core ------------------------------

enum Finished {
    /// For a note from the Glass.
    Speech(SpeechOutcome),
    /// The recording of a note taken on this phone.
    PhoneTranscript(NoteId, u16, Result<String, SpeechError>),
}

struct Link {
    machine: CompanionLink,
    decoder: FrameDecoder,
    profile: LinkProfile,
    notes_sync: bool,
}

struct Inner {
    store: Store,
    device_name: String,
    app_version: String,
    from_glass: CompanionNotes,
    from_phone: GlassNotes,
    link: Option<Link>,
    glass: GlassState,
    outgoing: Vec<u8>,
    events: Vec<CoreEvent>,
    close_link: bool,
}

/// See the crate docs.
#[derive(uniffi::Object)]
pub struct CompanionCore {
    inner: Mutex<Inner>,
    engines: Arc<Engines>,
    done_tx: Sender<Finished>,
    done_rx: Mutex<Receiver<Finished>>,
}

fn storage<E: std::fmt::Display>(e: E) -> CoreFailure {
    CoreFailure::Storage(e.to_string())
}

fn end_reason(end: &LinkEnd) -> String {
    match end {
        LinkEnd::UnknownPeer => "bonded to someone else".into(),
        LinkEnd::NotPaired => "not paired, and pairing is not allowed".into(),
        LinkEnd::AuthFailed => "authentication failed".into(),
        LinkEnd::PeerNotEncrypted => "the Glass does not encrypt the link: update it".into(),
        LinkEnd::Tampered => "a frame failed authentication: the link was tampered with".into(),
        LinkEnd::VersionMismatch { ours, theirs } => {
            format!("protocol version {theirs}, this app speaks {ours}")
        }
        LinkEnd::Protocol(s) => s.clone(),
    }
}

fn defer_reason(r: &NoteDeferReason) -> String {
    match r {
        NoteDeferReason::InsufficientStorage => "not enough storage on this phone".into(),
        NoteDeferReason::NeedsFasterLink { bytes } => {
            format!("an attachment of {bytes} bytes waits for a faster link")
        }
        NoteDeferReason::IntegrityFailure => "repeated checksum failures".into(),
    }
}

fn transcript_error(e: &NoteTranscriptError) -> String {
    match e {
        NoteTranscriptError::AudioNotAvailable => "the recording is not here yet".into(),
        NoteTranscriptError::NotAVoiceRecording => "not a voice recording".into(),
        NoteTranscriptError::EngineUnavailable => "no speech-to-text engine on this phone".into(),
        NoteTranscriptError::EngineFailed(s) => s.clone(),
    }
}

#[uniffi::export]
impl CompanionCore {
    /// Opens or creates the state under `data_dir`, an app-private directory. `device_name` is
    /// shown on the Glass.
    #[uniffi::constructor]
    pub fn new(
        data_dir: String,
        device_name: String,
        app_version: String,
        stt: Arc<dyn SpeechToTextEngine>,
        tts: Arc<dyn TextToSpeechEngine>,
    ) -> Result<Arc<CompanionCore>, CoreFailure> {
        let root = PathBuf::from(data_dir);
        std::fs::create_dir_all(&root).map_err(storage)?;
        let store = Store::load_or_create(root.join(STATE_FILE)).map_err(storage)?;
        let from_glass = CompanionNotes::open(root.join(GLASS_NOTES_DIR), RetryPolicy::default())
            .map_err(storage)?;
        let from_phone = GlassNotes::open(root.join(PHONE_NOTES_DIR), SenderConfig::default())
            .map_err(storage)?;
        let (done_tx, done_rx) = channel();
        Ok(Arc::new(CompanionCore {
            inner: Mutex::new(Inner {
                store,
                device_name,
                app_version,
                from_glass,
                from_phone,
                link: None,
                glass: GlassState::default(),
                outgoing: Vec::new(),
                events: Vec::new(),
                close_link: false,
            }),
            engines: Arc::new(Engines { stt, tts }),
            done_tx,
            done_rx: Mutex::new(done_rx),
        }))
    }

    /// The socket is open: start the handshake. With `allow_pairing`, an unbonded core bonds to
    /// the Glass at the other end, so set it only when the owner asked to pair.
    pub fn link_up(&self, profile: LinkProfile, allow_pairing: bool) {
        let mut inner = self.lock();
        let mode = if allow_pairing {
            PairingMode::AllowNew
        } else {
            PairingMode::ReuseOnly
        };
        let mut machine = CompanionLink::new(
            inner.store.identity().clone(),
            inner.store.paired().cloned(),
            mode,
            inner.device_name.clone(),
            inner.app_version.clone(),
            capabilities(),
        );
        let start = machine.start();
        inner.close_link = false;
        inner.outgoing.clear();
        inner.glass = GlassState::default();
        inner.link = Some(Link {
            machine,
            decoder: FrameDecoder::new(),
            profile,
            notes_sync: false,
        });
        inner.carry_out(start);
    }

    /// Bytes arrived from the socket.
    pub fn on_bytes(&self, data: Vec<u8>) {
        let mut inner = self.lock();
        let Some(link) = &mut inner.link else {
            return;
        };
        link.decoder.push(&data);
        let mut frames = Vec::new();
        let mut bad = None;
        loop {
            match link.decoder.next_frame() {
                Ok(Some(frame)) => frames.push(frame),
                Ok(None) => break,
                Err(e) => {
                    bad = Some(e.to_string());
                    break;
                }
            }
        }
        for frame in frames {
            inner.on_frame(frame);
            self.start_speech(&mut inner);
        }
        if let Some(reason) = bad {
            inner.link_failed(reason);
        }
    }

    /// The socket is closed, whatever the reason. Everything durable stays, and the Glass
    /// resumes on the next link.
    pub fn link_down(&self) {
        let mut inner = self.lock();
        inner.link_gone();
    }

    /// What the app must write and show, and time passing. Call from one thread, soon after any
    /// other call and at least every 250 ms while a link is up.
    pub fn pump(&self) -> CoreOutput {
        let finished: Vec<Finished> = {
            let done = self.done_rx.lock().unwrap_or_else(|p| p.into_inner());
            done.try_iter().collect()
        };
        let mut inner = self.lock();
        for f in finished {
            inner.finished(f);
        }
        inner.tick();
        self.start_speech(&mut inner);
        CoreOutput {
            outgoing: std::mem::take(&mut inner.outgoing),
            events: std::mem::take(&mut inner.events),
            close_link: std::mem::take(&mut inner.close_link),
        }
    }

    /// A snapshot for a UI.
    pub fn snapshot(&self) -> CoreSnapshot {
        let inner = self.lock();
        CoreSnapshot {
            phase: match &inner.link {
                None => LinkPhase::Down,
                Some(l) if l.machine.is_running() => LinkPhase::Running,
                Some(_) => LinkPhase::Handshake,
            },
            bonded_to: inner.store.paired().map(|p| p.peer_name.clone()),
            identity: inner.store.identity().public_b64(),
            glass: inner.glass.clone(),
        }
    }

    /// Asks the Glass for its settings and state.
    pub fn refresh_glass(&self) {
        let mut inner = self.lock();
        inner.ask_glass();
    }

    /// Changes the settings that are set in `change`.
    pub fn change_settings(&self, change: GlassSettings) {
        self.lock().send(&Frame::SettingsSet((&change).into()));
    }

    /// Switches a radio on the Glass.
    pub fn set_radio(&self, radio: GlassRadio, on: bool) {
        let radio = match radio {
            GlassRadio::Wifi => Radio::Wifi,
            GlassRadio::Bluetooth => Radio::Bt,
        };
        self.lock().send(&Frame::SetRadio(SetRadio { radio, on }));
    }

    /// Switches airplane mode on the Glass.
    pub fn set_airplane_mode(&self, on: bool) {
        self.lock()
            .send(&Frame::SetAirplaneMode(SetAirplaneMode { on }));
    }

    /// Sets the Glass's UI language.
    pub fn set_locale(&self, locale: NoteLocale) {
        self.lock().send(&Frame::SetLocale(SetLocale {
            locale: locale.into(),
        }));
    }

    /// Forgets the bonded Glass, and tells it to forget this phone if a link is up.
    pub fn unpair(&self) -> Result<(), CoreFailure> {
        let mut inner = self.lock();
        inner.send(&Frame::Unpair);
        inner.close_link = inner.link.is_some();
        inner.store.clear_paired().map_err(storage)
    }

    /// Free space where notes are stored, so a note that cannot fit is deferred up front.
    pub fn set_free_bytes(&self, free_bytes: Option<u64>) {
        self.lock().from_glass.set_free_bytes(free_bytes);
    }

    /// All notes, newest first.
    pub fn notes(&self) -> Vec<NoteInfo> {
        let inner = self.lock();
        let mut all: Vec<NoteInfo> = inner
            .from_glass
            .machine()
            .notes()
            .map(|n| note_from_glass(&inner, n))
            .collect();
        all.extend(
            inner
                .from_phone
                .machine()
                .notes()
                .into_iter()
                .map(|n| note_from_phone(&inner, n)),
        );
        all.sort_by_key(|n| std::cmp::Reverse(n.created_at_ms));
        all
    }

    /// Takes a note on this phone. It stays on this phone; only notes from the Glass sync, in
    /// the one direction the protocol has. Blocking while the files are copied and hashed: call
    /// it away from the UI thread. Returns the note's id.
    pub fn add_phone_note(
        &self,
        text: Option<String>,
        locale: NoteLocale,
        attachments: Vec<NewAttachment>,
    ) -> Result<String, CoreFailure> {
        let draft = NoteDraft {
            created_at_ms: now_ms(),
            locale: locale.into(),
            text: text.filter(|t| !t.is_empty()),
            attachments: attachments
                .into_iter()
                .map(|a| AttachmentSource {
                    kind: a.kind.into(),
                    mime: a.mime,
                    path: PathBuf::from(a.path),
                    duration_ms: a.duration_ms,
                })
                .collect(),
        };
        let sealer = self.lock().from_phone.sealer();
        let record = sealer
            .seal(&draft)
            .map_err(|e| CoreFailure::InvalidNote(e.to_string()))?;
        let id = record.manifest.id;
        let voice = record
            .manifest
            .attachments
            .iter()
            .find(|a| a.kind == AttachmentKind::VoiceRecording)
            .map(|a| (a.id, a.mime.clone()));
        let locale = record.manifest.locale;
        let mut inner = self.lock();
        // No link is ever up on this queue, so there are no frames to send.
        inner
            .from_phone
            .enqueue_sealed(record, now_ms())
            .map_err(storage)?;
        inner.events.push(CoreEvent::NotesChanged);
        if let Some((attachment_id, mime)) = voice {
            let request = TranscribeRequest {
                note_id: id,
                attachment_id,
                audio_path: inner.from_phone.attachment_path(&id, attachment_id),
                mime,
                locale,
            };
            drop(inner);
            let (engines, done) = (self.engines.clone(), self.done_tx.clone());
            std::thread::spawn(move || {
                let result = engines.transcribe(&request);
                let _ = done.send(Finished::PhoneTranscript(id, attachment_id, result));
            });
        }
        Ok(id.to_hex())
    }
}

impl CompanionCore {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A panic elsewhere must not take the whole app down with a poisoned lock: everything
        // that matters is on disk, and the driver reloads from there on an error.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Hands speech jobs to their own threads: they take far longer than a heartbeat.
    fn start_speech(&self, inner: &mut Inner) {
        for job in inner.from_glass.take_speech_jobs() {
            let (engines, done) = (self.engines.clone(), self.done_tx.clone());
            std::thread::spawn(move || {
                let outcome = run_job(&job, &engines);
                // The link may be gone: the job then runs again on the next one.
                let _ = done.send(Finished::Speech(outcome));
            });
        }
    }
}

fn run_job(job: &SpeechJob, engines: &Arc<Engines>) -> SpeechOutcome {
    let e = engines.as_ref();
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.run(e, e)))
        .unwrap_or_else(|_| job.failed("the speech engine crashed"))
}

fn capabilities() -> Vec<String> {
    capability::BACKBONE
        .iter()
        .chain([&capability::NOTES_SYNC])
        .map(|s| s.to_string())
        .collect()
}

fn note_from_glass(inner: &Inner, n: &IncomingNote) -> NoteInfo {
    let id = n.manifest.id;
    let transcript = n
        .transcripts
        .values()
        .find_map(|t| t.result.as_ref())
        .cloned();
    NoteInfo {
        id: id.to_hex(),
        origin: NoteOrigin::Glass,
        created_at_ms: n.manifest.created_at_ms,
        locale: n.manifest.locale.into(),
        text: n.manifest.text.clone(),
        transcript: transcript
            .as_ref()
            .filter(|t| t.error.is_none())
            .map(|t| t.text.clone()),
        transcript_error: transcript
            .as_ref()
            .and_then(|t| t.error.as_ref())
            .map(transcript_error),
        attachments: n
            .manifest
            .attachments
            .iter()
            .map(|a| NoteAttachmentInfo {
                kind: a.kind.into(),
                mime: a.mime.clone(),
                size: a.size,
                duration_ms: a.duration_ms,
                path: n
                    .attachments
                    .get(&a.id)
                    .is_some_and(|p| p.verified)
                    .then(|| {
                        inner
                            .from_glass
                            .attachment_path(&id, a.id)
                            .to_string_lossy()
                            .into_owned()
                    }),
            })
            .collect(),
        complete: n.complete,
    }
}

fn note_from_phone(inner: &Inner, n: &OutgoingNote) -> NoteInfo {
    let id = n.manifest.id;
    let transcript = n.transcripts.values().next();
    NoteInfo {
        id: id.to_hex(),
        origin: NoteOrigin::Phone,
        created_at_ms: n.manifest.created_at_ms,
        locale: n.manifest.locale.into(),
        text: n.manifest.text.clone(),
        transcript: transcript
            .filter(|t| t.error.is_none())
            .map(|t| t.text.clone()),
        transcript_error: transcript
            .and_then(|t| t.error.as_ref())
            .map(transcript_error),
        attachments: n
            .manifest
            .attachments
            .iter()
            .map(|a| NoteAttachmentInfo {
                kind: a.kind.into(),
                mime: a.mime.clone(),
                size: a.size,
                duration_ms: a.duration_ms,
                path: Some(
                    inner
                        .from_phone
                        .attachment_path(&id, a.id)
                        .to_string_lossy()
                        .into_owned(),
                ),
            })
            .collect(),
        complete: matches!(
            n.state,
            OutgoingState::Queued | OutgoingState::Synced { .. }
        ),
    }
}

impl Inner {
    fn send(&mut self, frame: &Frame) {
        // Frames can only be sent on a running link: the handshake sends its own.
        if !self.link.as_ref().is_some_and(|l| l.machine.is_running()) {
            return;
        }
        self.queue(frame);
    }

    fn queue(&mut self, frame: &Frame) {
        let Some(link) = &mut self.link else {
            return;
        };
        // Sealed here, under the lock, so the order on the socket is the order of the counter.
        match link.machine.encode(frame) {
            Ok(bytes) => self.outgoing.extend_from_slice(&bytes),
            // Only a frame over 16 MiB fails, and nothing here sends one.
            Err(e) => self.link_failed(format!("cannot encode {}: {e}", frame.wire_name())),
        }
    }

    fn carry_out(&mut self, outs: Vec<LinkOut>) {
        for out in outs {
            match out {
                LinkOut::Send(frame) => self.queue(&frame),
                LinkOut::Persist(rec) => {
                    let name = rec.peer_name.clone();
                    if let Err(e) = self.store.set_paired(rec.peer_pub, rec.peer_name) {
                        // Without the bond the next connection would fail auth anyway.
                        self.link_failed(format!("cannot store the bond: {e}"));
                        return;
                    }
                    self.events.push(CoreEvent::Paired { glass_name: name });
                }
                LinkOut::Running {
                    peer_name,
                    capabilities,
                } => self.running(peer_name, &capabilities),
                LinkOut::App(frame) => self.app_frame(frame),
                LinkOut::Closed(end) => {
                    self.events.push(CoreEvent::LinkClosed {
                        reason: end.err().as_ref().map(end_reason),
                    });
                    self.close_link = true;
                }
            }
        }
    }

    fn running(&mut self, glass_name: String, capabilities: &[String]) {
        let notes_sync = capabilities.iter().any(|c| c == capability::NOTES_SYNC);
        self.glass.name = Some(glass_name.clone());
        let mut profile = None;
        if let Some(link) = &mut self.link {
            link.notes_sync = notes_sync;
            profile = Some(link.profile);
        }
        self.events.push(CoreEvent::LinkRunning {
            glass_name,
            notes_sync,
        });
        self.ask_glass();
        if let (true, Some(profile)) = (notes_sync, profile) {
            let frames = self.from_glass.link_up(profile.into(), now_ms());
            self.note_frames(frames);
        }
    }

    fn ask_glass(&mut self) {
        self.send(&Frame::StatusRequest(StatusRequest {
            battery: true,
            storage: true,
            device: true,
            radios: true,
        }));
        self.send(&Frame::SettingsGet);
    }

    fn note_frames(&mut self, frames: Result<Vec<Frame>, omg_core::CoreError>) {
        match frames {
            Ok(frames) => frames.iter().for_each(|f| self.queue(f)),
            // The driver asks for this: it has reloaded from disk, so start over on a new link.
            Err(e) => self.link_failed(format!("note storage: {e}")),
        }
        self.note_events();
    }

    fn note_events(&mut self) {
        for event in self.from_glass.take_events() {
            let mapped = match event {
                ReceiverEvent::NoteComplete { note_id } => CoreEvent::NoteSynced {
                    note_id: note_id.to_hex(),
                },
                ReceiverEvent::NoteDeferred { note_id, reason } => CoreEvent::NoteDeferred {
                    note_id: note_id.to_hex(),
                    reason: defer_reason(&reason),
                },
                ReceiverEvent::NoteRejected { note_id, reason } => CoreEvent::NoteRefused {
                    note_id: note_id.to_hex(),
                    reason: match reason {
                        NoteRejectReason::InvalidManifest => "it breaks the note limits".into(),
                        NoteRejectReason::ManifestConflict => {
                            "a different note has the same id".into()
                        }
                    },
                },
                _ => CoreEvent::NotesChanged,
            };
            self.events.push(mapped);
            self.events.push(CoreEvent::NotesChanged);
        }
    }

    fn on_frame(&mut self, frame: Frame) {
        let Some(link) = &mut self.link else {
            return;
        };
        let outs = link.machine.on_frame(frame);
        self.carry_out(outs);
    }

    fn app_frame(&mut self, frame: Frame) {
        if omg_core::notes::is_note_frame(&frame) {
            if let Frame::NoteSyncStatus(NoteSyncStatus {
                pending_notes,
                pending_bytes,
                ..
            }) = &frame
            {
                self.glass.pending_notes = Some(*pending_notes);
                self.glass.pending_bytes = Some(*pending_bytes);
                self.events.push(CoreEvent::GlassStateChanged);
            }
            if self.link.as_ref().is_some_and(|l| l.notes_sync) {
                let frames = self.from_glass.on_frame(frame, now_ms());
                self.note_frames(frames);
            }
            return;
        }
        match frame {
            Frame::Settings(s) => {
                let settings = GlassSettings::from(&s);
                self.glass.settings = Some(settings.clone());
                if let Some(radios) = &s.radios {
                    self.radios(radios);
                }
                self.events.push(CoreEvent::SettingsChanged { settings });
            }
            Frame::Status(status) => self.status(status),
            Frame::BatteryStatus(BatteryStatus { percent, charging }) => {
                self.glass.battery_percent = Some(percent.into());
                self.glass.charging = Some(charging);
                self.events.push(CoreEvent::GlassStateChanged);
            }
            Frame::RadioState(radios) => {
                self.radios(&radios);
                self.events.push(CoreEvent::GlassStateChanged);
            }
            Frame::LocaleChanged(l) => {
                let settings = self.glass.settings.get_or_insert_with(Default::default);
                settings.locale = Some(l.locale.into());
                let settings = settings.clone();
                self.events.push(CoreEvent::SettingsChanged { settings });
            }
            _ => {}
        }
    }

    fn status(&mut self, s: GlassStatus) {
        let g = &mut self.glass;
        g.battery_percent = s.battery_percent.map(u32::from).or(g.battery_percent);
        g.charging = s.charging.or(g.charging);
        g.software_version = s.software_version.or(g.software_version.take());
        g.storage_free_bytes = s.storage_free_bytes.or(g.storage_free_bytes);
        if let Some(name) = s.device_name {
            g.name = Some(name);
        }
        if let Some(radios) = &s.radios {
            self.radios(radios);
        }
        self.events.push(CoreEvent::GlassStateChanged);
    }

    fn radios(&mut self, r: &RadioState) {
        self.glass.wifi_on = Some(r.wifi_on);
        self.glass.bluetooth_on = Some(r.bt_on);
        self.glass.airplane_mode = Some(r.airplane_mode);
    }

    fn tick(&mut self) {
        if self.link.as_ref().is_some_and(|l| l.notes_sync) {
            let frames = self.from_glass.tick(now_ms());
            self.note_frames(frames);
        }
    }

    fn finished(&mut self, f: Finished) {
        match f {
            Finished::Speech(outcome) => {
                let frames = self.from_glass.complete_speech(outcome, now_ms());
                // Without a link the frames are dropped: the Glass asks again on the next one.
                if self.link.as_ref().is_some_and(|l| l.notes_sync) {
                    self.note_frames(frames);
                } else {
                    self.note_events();
                }
            }
            Finished::PhoneTranscript(note_id, attachment_id, result) => {
                let (text, error) = match result {
                    Ok(text) => (text, None),
                    Err(SpeechError::Unavailable) => {
                        (String::new(), Some(NoteTranscriptError::EngineUnavailable))
                    }
                    Err(SpeechError::Failed(why)) => {
                        (String::new(), Some(NoteTranscriptError::EngineFailed(why)))
                    }
                };
                // The same path a transcript from the companion takes on the Glass; its
                // acknowledgement has nowhere to go here.
                let stored = self.from_phone.on_frame(
                    Frame::NoteTranscript(NoteTranscript {
                        note_id,
                        attachment_id,
                        text,
                        truncated: false,
                        error,
                    }),
                    now_ms(),
                );
                if let Err(e) = stored {
                    self.events.push(CoreEvent::NoteRefused {
                        note_id: note_id.to_hex(),
                        reason: format!("cannot store the transcript: {e}"),
                    });
                }
                self.events.push(CoreEvent::NotesChanged);
            }
        }
    }

    fn link_failed(&mut self, reason: String) {
        self.events.push(CoreEvent::LinkClosed {
            reason: Some(reason),
        });
        self.close_link = true;
    }

    fn link_gone(&mut self) {
        if self.link.take().is_some() {
            if let Err(e) = self.from_glass.link_down(now_ms()) {
                self.events.push(CoreEvent::LinkClosed {
                    reason: Some(format!("note storage: {e}")),
                });
            }
            self.note_events();
        }
    }
}
