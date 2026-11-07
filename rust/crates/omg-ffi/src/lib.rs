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
//! A note can change with no link at all, when speech finishes on this phone, and nothing pumps
//! then. So while the app is open, a third thread calls [`CompanionCore::poll`] now and then.
//!
//! Every call is safe from any thread, and **none belongs on a UI thread**: a call waits for the
//! core's lock, which the link's reader holds while it syncs a file or hashes a whole attachment,
//! for seconds on a big video. [`CompanionCore::add_phone_note`] also copies and hashes its files.
//!
//! One core per directory. A second one on the same directory is refused while the first is alive,
//! so an app keeps a single core for the life of its process. The guard only sees this process: an
//! iOS app extension sharing the directory through an App Group would not be stopped by it.

uniffi::setup_scaffolding!();

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Seek, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
    time::Duration,
};

use omg_core::{
    notes::{
        record::OutgoingState, split_for_speech, wav_header, AttachmentSource, CompanionNotes,
        GlassNotes, IncomingNote, NoteDraft, NoteTransfer as CoreNoteTransfer, OutgoingNote,
        ReceiverEvent, RetryPolicy, SenderConfig, SpeechError, SpeechJob, SpeechOutcome,
        SpeechToText, SynthesizeRequest, SynthesizedAudio, TextToSpeech, TranscribeRequest,
    },
    now_ms, CompanionLink, CoreError, LinkEnd, LinkOut, PairingMode, Store,
};
use omg_protocol::{
    capability, AttachmentKind, BatteryStatus, Frame, FrameDecoder, GlassStatus, NoteDeferReason,
    NoteId, NoteRejectReason, NoteSyncStatus, NoteTranscript, NoteTranscriptError, Radio,
    RadioState, SetAirplaneMode, SetLocale, SetRadio, Settings, StatusRequest,
};

const STATE_FILE: &str = "companion.json";
const GLASS_NOTES_DIR: &str = "notes-from-glass";
const PHONE_NOTES_DIR: &str = "notes-from-phone";
/// Audio made to read a note aloud. Nothing here outlives the app's next start.
const SPEECH_DIR: &str = "speech";
/// A speech engine gets at most this much text at a time. Android's limit is 4000 characters.
const SPEECH_PIECE_BYTES: usize = 3000;
/// Marks recordings being transcribed, so a transcription that takes the app down can be told
/// from one that was merely cut short.
const STT_ATTEMPTS_DIR: &str = "stt-attempts";
/// Starts of one transcription that may end without a result before it stops being resumed.
const MAX_STT_ATTEMPTS: u32 = 3;
const STT_GAVE_UP: &str = "transcribing it kept stopping the app";
/// A recognizer another app is using gets this many tries, this far apart.
const STT_BUSY_TRIES: u32 = 3;
const STT_BUSY_WAIT: Duration = Duration::from_secs(1);
const STT_BUSY: &str = "the speech recognizer is busy";
/// A WAV file cannot say it holds more.
const MAX_WAV_DATA_BYTES: u64 = u32::MAX as u64 - 44;
/// How often a transfer reports its progress.
const PROGRESS_INTERVAL_MS: u64 = 1000;

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
    /// This needs the Glass, over the link named, and that link is not up and running.
    #[error("not connected: {0}")]
    NotConnected(String),
}

/// Why a speech engine produced nothing.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SpeechFailure {
    /// No engine here, or none for this language.
    #[error("no speech engine available")]
    Unavailable,
    /// The engine is there, and in use by someone else for now. The core tries again.
    #[error("the speech engine is busy")]
    Busy,
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
            SpeechFailure::Busy => SpeechError::Failed(STT_BUSY.into()),
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

/// How long to wait before trying the Glass again. One per supervised link. See
/// `omg_core::reconnect`.
#[derive(uniffi::Object)]
pub struct ReconnectBackoff {
    inner: Mutex<omg_core::reconnect::Backoff>,
}

impl Default for ReconnectBackoff {
    fn default() -> Self {
        ReconnectBackoff {
            inner: Mutex::new(omg_core::reconnect::Backoff::new()),
        }
    }
}

#[uniffi::export]
impl ReconnectBackoff {
    #[uniffi::constructor]
    pub fn new() -> Arc<ReconnectBackoff> {
        Arc::new(ReconnectBackoff::default())
    }

    /// An attempt ended, after a link of `lasted_ms`, or 0 if none opened. How long to wait now.
    pub fn next_wait_ms(&self, lasted_ms: u64) -> u32 {
        self.lock().next_wait_ms(lasted_ms)
    }

    /// The owner asked for a link: the next failure starts from the shortest wait again.
    pub fn reset(&self) {
        self.lock().reset();
    }
}

impl ReconnectBackoff {
    fn lock(&self) -> std::sync::MutexGuard<'_, omg_core::reconnect::Backoff> {
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

/// The language for a note taken on this phone when the Glass has not said which: the phone's
/// own, from its ISO 639-1 code, or the default if it is not one the project supports.
#[uniffi::export]
pub fn note_locale_for_language(code: String) -> NoteLocale {
    omg_protocol::Locale::from_code(&code)
        .unwrap_or_default()
        .into()
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

/// How far the note being received has come.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NoteTransfer {
    /// The note.
    pub note_id: String,
    /// Bytes here so far.
    pub received_bytes: u64,
    /// Bytes in all its attachments.
    pub total_bytes: u64,
}

impl From<CoreNoteTransfer> for NoteTransfer {
    fn from(t: CoreNoteTransfer) -> Self {
        NoteTransfer {
            note_id: t.note_id.to_hex(),
            received_bytes: t.received_bytes,
            total_bytes: t.total_bytes,
        }
    }
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
    /// The note being received right now.
    pub receiving: Option<NoteTransfer>,
    /// Notes whose records could not be read. They are kept aside on disk and not listed.
    pub notes_set_aside: u32,
    /// The link up now, whatever its phase, for [`CompanionCore::unpair`].
    pub link_id: Option<u64>,
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
    /// The transfer moved on: read `receiving` in [`CompanionCore::snapshot`].
    NoteProgress,
    /// The audio [`CompanionCore::speak_note`] asked for. Play it from `path`, until the next
    /// call to `speak_note` or `stop_speaking`, which remove the file.
    NoteSpeechReady {
        /// What the caller passed to `speak_note`.
        request_id: u64,
        /// The note.
        note_id: String,
        /// A WAV file.
        path: String,
    },
    /// [`CompanionCore::speak_note`] produced nothing.
    NoteSpeechFailed {
        /// What the caller passed to `speak_note`.
        request_id: u64,
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
    /// Speech-to-text is running on its recording.
    pub transcribing: bool,
    /// What the note is waiting for, if it is not all here. For the owner.
    pub waiting: Option<String>,
    /// [`CompanionCore::delete_note`] would take it. One still arriving would only be offered
    /// again by the Glass.
    pub deletable: bool,
    /// There is text for [`CompanionCore::speak_note`] to read.
    pub speakable: bool,
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
    /// Speak `text`, at most a few thousand bytes of it.
    fn synthesize(&self, text: String, locale: NoteLocale) -> Result<SpeechAudio, SpeechFailure>;

    /// Called from another thread: make a `synthesize` in progress return now, with any error.
    /// Doing nothing is allowed, and only costs the wait for that piece.
    ///
    /// In a narrow race, when the owner stops just as reading a note ends, this can land on the
    /// next job instead, which may be one the Glass asked for: that one then fails, and the Glass
    /// may ask again. Speech-to-text has no cancel and always runs to its end.
    fn cancel(&self);
}

struct Engines {
    stt: Arc<dyn SpeechToTextEngine>,
    tts: Arc<dyn TextToSpeechEngine>,
}

impl SpeechToText for Engines {
    fn transcribe(&self, request: &TranscribeRequest) -> Result<String, SpeechError> {
        let mut tries = 1;
        loop {
            let result = self.stt.transcribe(
                request.audio_path.to_string_lossy().into_owned(),
                request.mime.clone(),
                request.locale.into(),
            );
            if !matches!(result, Err(SpeechFailure::Busy)) || tries >= STT_BUSY_TRIES {
                return result.map_err(Into::into);
            }
            tries += 1;
            std::thread::sleep(STT_BUSY_WAIT);
        }
    }
}

impl TextToSpeech for Engines {
    /// Long text goes to the engine in pieces, and the audio comes back as one.
    fn synthesize(&self, request: &SynthesizeRequest) -> Result<SynthesizedAudio, SpeechError> {
        let mut all: Option<SynthesizedAudio> = None;
        for piece in split_for_speech(&request.text, SPEECH_PIECE_BYTES) {
            let audio = self.tts.synthesize(piece.into(), request.locale.into())?;
            match &mut all {
                None => {
                    all = Some(SynthesizedAudio {
                        sample_rate: audio.sample_rate,
                        pcm: audio.pcm,
                    })
                }
                Some(all) if all.sample_rate == audio.sample_rate => {
                    all.pcm.extend_from_slice(&audio.pcm)
                }
                Some(_) => {
                    return Err(SpeechError::Failed(
                        "the engine changed its sample rate".into(),
                    ))
                }
            }
        }
        all.ok_or_else(|| SpeechError::Failed("nothing to say".into()))
    }
}

// ------------------------------ the core ------------------------------

enum Finished {
    /// For a note from the Glass.
    Speech(SpeechOutcome),
    /// The recording of a note taken on this phone.
    PhoneTranscript(NoteId, u16, Result<String, SpeechError>),
    /// A note read aloud on this phone.
    Spoken {
        request_id: u64,
        generation: u64,
        note_id: NoteId,
        result: Result<PathBuf, String>,
    },
}

/// Work for the one thread that runs speech-to-text: a phone's recognizer does one at a time.
enum SttJob {
    Glass(TranscribeRequest),
    Phone(TranscribeRequest),
}

/// Reading a note aloud on this phone.
struct PhoneSpeech {
    request_id: u64,
    generation: u64,
    request: SynthesizeRequest,
    path: PathBuf,
}

/// Work for the one thread that runs text-to-speech.
enum TtsJob {
    Glass(SpeechJob),
    Phone(PhoneSpeech),
}

/// Starts of each transcription, counted on disk, so one that takes the app down cannot do so on
/// every launch. Counted by the worker just before the engine, so waiting in the queue is free.
#[derive(Clone)]
struct SttAttempts {
    dir: PathBuf,
}

impl SttAttempts {
    fn marker(&self, id: NoteId, attachment: u16) -> PathBuf {
        self.dir.join(format!("{}-{attachment}", id.to_hex()))
    }

    /// Counts one more start. False once there have been too many without a result.
    fn begin(&self, id: NoteId, attachment: u16) -> bool {
        let marker = self.marker(id, attachment);
        let attempts: u32 = std::fs::read_to_string(&marker)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if attempts >= MAX_STT_ATTEMPTS {
            return false;
        }
        // If this fails the disk is full, and the count is the least of the problems.
        let _ = std::fs::write(&marker, (attempts + 1).to_string());
        true
    }

    /// A result came back: the count starts over.
    fn finished(&self, id: NoteId, attachment: u16) {
        remove_file(&self.marker(id, attachment));
    }

    fn forget(&self, id: NoteId) {
        let prefix = format!("{}-", id.to_hex());
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                remove_file(&entry.path());
            }
        }
    }
}

/// Directories with a live core.
static OPEN_DIRS: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// Holds a directory for one core: two would each overwrite what the other wrote.
struct DirClaim(PathBuf);

impl DirClaim {
    fn take(dir: &Path) -> Result<DirClaim, CoreFailure> {
        let dir = dir.canonicalize().map_err(storage)?;
        let mut open = OPEN_DIRS.lock().unwrap_or_else(|p| p.into_inner());
        if !open.insert(dir.clone()) {
            return Err(CoreFailure::Storage(format!(
                "{} is already open in this process",
                dir.display()
            )));
        }
        Ok(DirClaim(dir))
    }
}

impl Drop for DirClaim {
    fn drop(&mut self) {
        let mut open = OPEN_DIRS.lock().unwrap_or_else(|p| p.into_inner());
        open.remove(&self.0);
    }
}

struct Link {
    /// Counts links over the life of the core, for the app to name one.
    id: u64,
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
    /// Why notes from the Glass are waiting, since the last link came up.
    deferred: BTreeMap<NoteId, String>,
    /// Recordings queued for speech-to-text or in it, on either side.
    stt_running: BTreeSet<(NoteId, u16)>,
    stt_attempts: SttAttempts,
    speech_dir: PathBuf,
    /// Only the latest `speak_note` counts. Shared with the thread that runs it.
    speech_generation: Arc<AtomicU64>,
    speech_file: Option<PathBuf>,
    last_progress: Option<(CoreNoteTransfer, u64)>,
    /// The id of the last link brought up.
    last_link_id: u64,
}

/// See the crate docs.
#[derive(uniffi::Object)]
pub struct CompanionCore {
    inner: Mutex<Inner>,
    engines: Arc<Engines>,
    stt_tx: Sender<SttJob>,
    tts_tx: Sender<TtsJob>,
    /// The generation being read aloud right now, or 0.
    speaking_now: Arc<AtomicU64>,
    _claim: DirClaim,
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

fn parse_note_id(id: &str) -> Result<NoteId, CoreFailure> {
    NoteId::from_hex(id).ok_or_else(no_such_note)
}

/// What reading a note aloud says: its text, then what was heard, unless they are the same.
fn read_aloud_text(text: Option<&str>, transcript: Option<&str>) -> Option<String> {
    let text = text.map(str::trim).filter(|t| !t.is_empty());
    let heard = transcript.map(str::trim).filter(|t| !t.is_empty());
    match (text, heard) {
        (Some(t), Some(h)) if t != h => Some(format!("{t}\n{h}")),
        (Some(t), _) => Some(t.to_string()),
        (None, Some(h)) => Some(h.to_string()),
        (None, None) => None,
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
        let claim = DirClaim::take(&root)?;
        let store = Store::load_or_create(root.join(STATE_FILE)).map_err(storage)?;
        let from_glass = CompanionNotes::open(root.join(GLASS_NOTES_DIR), RetryPolicy::default())
            .map_err(storage)?;
        let from_phone = GlassNotes::open(root.join(PHONE_NOTES_DIR), SenderConfig::default())
            .map_err(storage)?;
        let speech_dir = root.join(SPEECH_DIR);
        // What is left here belongs to a run of the app that is over.
        match std::fs::remove_dir_all(&speech_dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(storage(e)),
            _ => {}
        }
        std::fs::create_dir_all(&speech_dir).map_err(storage)?;
        let stt_attempts = SttAttempts {
            dir: root.join(STT_ATTEMPTS_DIR),
        };
        std::fs::create_dir_all(&stt_attempts.dir).map_err(storage)?;
        let (done_tx, done_rx) = channel();
        let engines = Arc::new(Engines { stt, tts });
        let speech_generation = Arc::new(AtomicU64::new(0));
        let speaking_now = Arc::new(AtomicU64::new(0));
        let stt_tx = spawn_stt_worker(engines.clone(), done_tx.clone(), stt_attempts.clone());
        let tts_tx = spawn_tts_worker(
            engines.clone(),
            done_tx.clone(),
            speech_generation.clone(),
            speaking_now.clone(),
        );
        let core = Arc::new(CompanionCore {
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
                deferred: BTreeMap::new(),
                stt_running: BTreeSet::new(),
                stt_attempts,
                speech_dir,
                speech_generation,
                speech_file: None,
                last_progress: None,
                last_link_id: 0,
            }),
            engines,
            stt_tx,
            tts_tx,
            speaking_now,
            _claim: claim,
            done_rx: Mutex::new(done_rx),
        });
        core.resume_phone_transcripts();
        Ok(core)
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
        inner.last_link_id += 1;
        inner.link = Some(Link {
            id: inner.last_link_id,
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
        let mut inner = self.finish_speech();
        inner.tick();
        self.start_speech(&mut inner);
        CoreOutput {
            outgoing: std::mem::take(&mut inner.outgoing),
            events: std::mem::take(&mut inner.events),
            close_link: std::mem::take(&mut inner.close_link),
        }
    }

    /// What happened while no link is up. Results of finished speech work are stored here, so a
    /// transcript taken with no link still shows up. Call now and then from any thread while the
    /// app is open. **While a link is up it returns nothing:** everything then goes to
    /// [`pump`](CompanionCore::pump), so link events keep their order and reach the caller that
    /// knows which link they came from. It never touches what `pump` must write.
    pub fn poll(&self) -> Vec<CoreEvent> {
        let mut inner = self.finish_speech();
        self.start_speech(&mut inner);
        // Under the same lock as the check, so no link event can slip in between.
        if inner.link.is_some() {
            return Vec::new();
        }
        std::mem::take(&mut inner.events)
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
            receiving: inner.from_glass.machine().transfer().map(Into::into),
            notes_set_aside: (inner.from_glass.set_aside().len()
                + inner.from_phone.set_aside().len()) as u32,
            link_id: inner.link.as_ref().map(|l| l.id),
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

    /// Forgets the bonded Glass and tells it to forget this phone, over the link `link_id` names,
    /// as [`CoreSnapshot::link_id`] gave it. That link must be up and past its handshake: the phone
    /// never forgets while the Glass is not told, and an Unpair asked for one link never ends a
    /// newer one. Anything else is [`CoreFailure::NotConnected`], and nothing changes. If this
    /// phone cannot forget, nothing is sent and the link stays: the two ends keep agreeing.
    ///
    /// The link then closes. The app must not open another by itself, or it would pair again: a
    /// new bond needs the owner to ask.
    pub fn unpair(&self, link_id: u64) -> Result<(), CoreFailure> {
        let mut inner = self.lock();
        match &inner.link {
            Some(link) if link.id == link_id && link.machine.is_running() => {}
            Some(link) if link.id == link_id => {
                return Err(CoreFailure::NotConnected(
                    "the Glass is still connecting, try again in a moment".into(),
                ))
            }
            Some(_) => {
                return Err(CoreFailure::NotConnected(
                    "the connection changed since, try again".into(),
                ))
            }
            None => {
                return Err(CoreFailure::NotConnected(
                    "connect to the Glass first, so it forgets this phone too".into(),
                ))
            }
        }
        inner.store.clear_paired().map_err(storage)?;
        inner.send(&Frame::Unpair);
        inner.close_link = true;
        Ok(())
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
        let record = sealer.seal(&draft).map_err(|e| match e {
            // It would otherwise say "invalid note" twice.
            CoreError::InvalidNote(why) => CoreFailure::InvalidNote(why),
            other => CoreFailure::InvalidNote(other.to_string()),
        })?;
        let id = record.manifest.id;
        let mut inner = self.lock();
        // No link is ever up on this queue, so there are no frames to send.
        inner
            .from_phone
            .enqueue_sealed(record, now_ms())
            .map_err(storage)?;
        inner.events.push(CoreEvent::NotesChanged);
        self.start_phone_transcript(&mut inner, id);
        Ok(id.to_hex())
    }

    /// Removes a note from this phone, and its files. A note from the Glass must be all here,
    /// see [`NoteInfo::deletable`]. The Glass normally deleted its copy when this phone
    /// acknowledged it. If that acknowledgement was lost, the note syncs here again.
    pub fn delete_note(&self, note_id: String) -> Result<(), CoreFailure> {
        let id = parse_note_id(&note_id)?;
        let mut inner = self.lock();
        let deleted = if inner.from_phone.machine().note(&id).is_some() {
            inner.from_phone.forget(&id, now_ms()).map_err(storage)?
        } else if inner.from_glass.machine().note(&id).is_some() {
            inner.from_glass.forget(&id, now_ms()).map_err(|e| {
                // The driver has reloaded itself and no longer knows this link: as it asks, this
                // counts as losing the link, and the next one starts clean. With no link there is
                // nothing to end.
                if inner.link.is_some() {
                    inner.link_failed(format!("note storage: {e}"));
                }
                storage(e)
            })?
        } else {
            return Err(no_such_note());
        };
        if !deleted {
            return Err(CoreFailure::InvalidNote(
                "it is still arriving from the Glass".into(),
            ));
        }
        inner.deferred.remove(&id);
        inner.forget_stt_attempts(id);
        inner.events.push(CoreEvent::NotesChanged);
        Ok(())
    }

    /// Runs speech-to-text again on a recording that has no text: the engine failed, or there
    /// was none. The result arrives as [`CoreEvent::NotesChanged`].
    pub fn retranscribe_note(&self, note_id: String) -> Result<(), CoreFailure> {
        let id = parse_note_id(&note_id)?;
        let mut inner = self.lock();
        // The owner asking is worth a fresh count, whatever happened before.
        inner.forget_stt_attempts(id);
        if inner.from_phone.machine().note(&id).is_some() {
            self.start_phone_transcript(&mut inner, id);
        } else if inner.from_glass.machine().note(&id).is_some() {
            let frames = inner.from_glass.retranscribe(&id, now_ms());
            inner.note_frames(frames);
            self.start_speech(&mut inner);
        } else {
            return Err(no_such_note());
        }
        inner.events.push(CoreEvent::NotesChanged);
        Ok(())
    }

    /// Reads a note aloud on this phone, with no link needed. The audio arrives as
    /// [`CoreEvent::NoteSpeechReady`] or the reason why not as [`CoreEvent::NoteSpeechFailed`],
    /// carrying `request_id`, so a caller can tell an answer to an earlier request. Replaces any
    /// earlier request, and what it was reading stops.
    pub fn speak_note(&self, note_id: String, request_id: u64) -> Result<(), CoreFailure> {
        let id = parse_note_id(&note_id)?;
        let mut inner = self.lock();
        let note = inner.note_info(&id).ok_or_else(no_such_note)?;
        let text = read_aloud_text(note.text.as_deref(), note.transcript.as_deref())
            .ok_or_else(|| CoreFailure::InvalidNote("this note has nothing to read".into()))?;
        let generation = inner.stop_speaking();
        let path = inner.speech_dir.join(format!("{generation}.wav"));
        drop(inner);
        self.cancel_running_speech();
        let _ = self.tts_tx.send(TtsJob::Phone(PhoneSpeech {
            request_id,
            generation,
            request: SynthesizeRequest {
                note_id: id,
                text,
                locale: note.locale.into(),
            },
            path,
        }));
        Ok(())
    }

    /// Forgets what [`speak_note`](CompanionCore::speak_note) asked for or made, and stops the
    /// engine if it is at it.
    pub fn stop_speaking(&self) {
        self.lock().stop_speaking();
        self.cancel_running_speech();
    }
}

impl CompanionCore {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A panic elsewhere must not take the whole app down with a poisoned lock: everything
        // that matters is on disk, and the driver reloads from there on an error.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Stores the results of speech work that has finished.
    fn finish_speech(&self) -> std::sync::MutexGuard<'_, Inner> {
        let finished: Vec<Finished> = {
            let done = self.done_rx.lock().unwrap_or_else(|p| p.into_inner());
            done.try_iter().collect()
        };
        let mut inner = self.lock();
        for f in finished {
            inner.finished(f);
        }
        inner
    }

    /// Queues the recording of a phone note for speech-to-text, unless it already has text or
    /// is already on its way.
    fn start_phone_transcript(&self, inner: &mut Inner, id: NoteId) {
        let Some(note) = inner.from_phone.machine().note(&id) else {
            return;
        };
        let Some(voice) = note
            .manifest
            .attachments
            .iter()
            .find(|a| a.kind == AttachmentKind::VoiceRecording)
        else {
            return;
        };
        let has_text = note
            .transcripts
            .get(&voice.id)
            .is_some_and(|t| t.error.is_none());
        if has_text {
            return;
        }
        let attachment_id = voice.id;
        let request = TranscribeRequest {
            note_id: id,
            attachment_id,
            audio_path: inner.from_phone.attachment_path(&id, attachment_id),
            mime: voice.mime.clone(),
            locale: note.manifest.locale,
        };
        if inner.stt_running.insert((id, attachment_id)) {
            let _ = self.stt_tx.send(SttJob::Phone(request));
        }
    }

    /// A recording with neither text nor a reason means the app stopped while transcribing it.
    /// One with a reason waits for the owner to ask again, or it would run on every start.
    fn resume_phone_transcripts(&self) {
        let mut inner = self.lock();
        let unfinished: Vec<NoteId> = inner
            .from_phone
            .machine()
            .notes()
            .into_iter()
            .filter(|n| n.transcripts.is_empty())
            .map(|n| n.manifest.id)
            .collect();
        for id in unfinished {
            self.start_phone_transcript(&mut inner, id);
        }
    }

    /// Hands speech jobs to their threads: they take far longer than a heartbeat.
    fn start_speech(&self, inner: &mut Inner) {
        for job in inner.from_glass.take_speech_jobs() {
            let SpeechJob::Transcribe(request) = job else {
                let _ = self.tts_tx.send(TtsJob::Glass(job));
                continue;
            };
            if inner
                .stt_running
                .insert((request.note_id, request.attachment_id))
            {
                let _ = self.stt_tx.send(SttJob::Glass(request));
            }
        }
    }

    /// Stops the engine if it is reading aloud. The generation has already moved on.
    fn cancel_running_speech(&self) {
        if self.speaking_now.load(Ordering::SeqCst) != 0 {
            self.engines.tts.cancel();
        }
    }
}

fn run_job(job: &SpeechJob, engines: &Arc<Engines>) -> SpeechOutcome {
    let e = engines.as_ref();
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.run(e, e)))
        .unwrap_or_else(|_| job.failed("the speech engine crashed"))
}

/// The thread for speech-to-text. It ends when the core is gone.
fn spawn_stt_worker(
    engines: Arc<Engines>,
    done: Sender<Finished>,
    attempts: SttAttempts,
) -> Sender<SttJob> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for job in rx {
            let (note_id, attachment_id) = match &job {
                SttJob::Glass(r) | SttJob::Phone(r) => (r.note_id, r.attachment_id),
            };
            let run = attempts.begin(note_id, attachment_id);
            let finished = match job {
                SttJob::Glass(r) if !run => {
                    Finished::Speech(SpeechJob::Transcribe(r).failed(STT_GAVE_UP))
                }
                SttJob::Phone(_) if !run => Finished::PhoneTranscript(
                    note_id,
                    attachment_id,
                    Err(SpeechError::Failed(STT_GAVE_UP.into())),
                ),
                // The link may be gone: the result is stored all the same.
                SttJob::Glass(r) => Finished::Speech(run_job(&SpeechJob::Transcribe(r), &engines)),
                SttJob::Phone(request) => {
                    let e = engines.as_ref();
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        e.transcribe(&request)
                    }))
                    .unwrap_or_else(|_| {
                        Err(SpeechError::Failed("the speech engine crashed".into()))
                    });
                    Finished::PhoneTranscript(request.note_id, request.attachment_id, result)
                }
            };
            let _ = done.send(finished);
        }
    });
    tx
}

/// The thread for text-to-speech. Requests replaced while they waited here are skipped.
fn spawn_tts_worker(
    engines: Arc<Engines>,
    done: Sender<Finished>,
    latest: Arc<AtomicU64>,
    speaking_now: Arc<AtomicU64>,
) -> Sender<TtsJob> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for job in rx {
            let job = match job {
                TtsJob::Glass(job) => {
                    let _ = done.send(Finished::Speech(run_job(&job, &engines)));
                    continue;
                }
                TtsJob::Phone(job) => job,
            };
            // Set before the check: a stop after the check sees this and cancels the engine.
            speaking_now.store(job.generation, Ordering::SeqCst);
            if latest.load(Ordering::SeqCst) == job.generation {
                let result = speak(&engines, &job, &latest);
                speaking_now.store(0, Ordering::SeqCst);
                let _ = done.send(Finished::Spoken {
                    request_id: job.request_id,
                    generation: job.generation,
                    note_id: job.request.note_id,
                    result,
                });
            }
            speaking_now.store(0, Ordering::SeqCst);
        }
    });
    tx
}

/// Synthesizes into a WAV file a piece at a time, so a long note neither sits in memory nor runs
/// on after it stopped being wanted. Leaves no file behind unless it succeeds. Blocking.
fn speak(engines: &Arc<Engines>, job: &PhoneSpeech, latest: &AtomicU64) -> Result<PathBuf, String> {
    let result = write_speech(engines, job, latest);
    if result.is_err() {
        remove_file(&job.path);
    }
    result.map(|()| job.path.clone())
}

fn write_speech(
    engines: &Arc<Engines>,
    job: &PhoneSpeech,
    latest: &AtomicU64,
) -> Result<(), String> {
    let mut file = std::fs::File::create(&job.path).map_err(|e| e.to_string())?;
    file.write_all(&wav_header(0, 0))
        .map_err(|e| e.to_string())?;
    let mut sample_rate = None;
    let mut written = 0u64;
    for piece in split_for_speech(&job.request.text, SPEECH_PIECE_BYTES) {
        if latest.load(Ordering::SeqCst) != job.generation {
            return Err("stopped".into());
        }
        let tts = engines.tts.as_ref();
        let locale = job.request.locale.into();
        let audio = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tts.synthesize(piece.into(), locale)
        }))
        .unwrap_or_else(|_| {
            Err(SpeechFailure::Failed {
                reason: "the speech engine crashed".into(),
            })
        })
        .map_err(|e| match e {
            SpeechFailure::Unavailable => {
                "no text-to-speech engine on this phone for this language".to_string()
            }
            SpeechFailure::Busy => "the speech engine is busy".to_string(),
            SpeechFailure::Failed { reason } => reason,
        })?;
        if *sample_rate.get_or_insert(audio.sample_rate) != audio.sample_rate {
            return Err("the engine changed its sample rate".into());
        }
        written += audio.pcm.len() as u64;
        if written > MAX_WAV_DATA_BYTES {
            return Err("the audio is too long".into());
        }
        file.write_all(&audio.pcm).map_err(|e| e.to_string())?;
    }
    let sample_rate = sample_rate.unwrap_or(0);
    if written == 0 || sample_rate == 0 {
        return Err("the engine returned no audio".into());
    }
    file.rewind().map_err(|e| e.to_string())?;
    file.write_all(&wav_header(sample_rate, written as u32))
        .map_err(|e| e.to_string())
}

/// Whether a transfer has something new to report: at once at its start, then only if it moved,
/// and not more often than [`PROGRESS_INTERVAL_MS`].
fn progress_due(
    last: Option<(CoreNoteTransfer, u64)>,
    transfer: CoreNoteTransfer,
    now: u64,
) -> bool {
    last.is_none_or(|(last, at)| last != transfer && now.saturating_sub(at) >= PROGRESS_INTERVAL_MS)
}

fn no_such_note() -> CoreFailure {
    CoreFailure::InvalidNote("no such note".into())
}

fn remove_file(path: &Path) {
    // Gone already is fine, and a leftover goes on the next start.
    let _ = std::fs::remove_file(path);
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
        // The machine forgets a job when the link goes, and the job runs on.
        transcribing: inner.from_glass.machine().is_transcribing(&id) || inner.stt_is_running(id),
        waiting: if n.complete {
            None
        } else {
            inner.deferred.get(&id).cloned().or_else(|| {
                let running = inner.link.as_ref().is_some_and(|l| l.machine.is_running());
                (!running).then(|| "the rest arrives when the Glass connects".to_string())
            })
        },
        deletable: n.complete,
        speakable: read_aloud_text(
            n.manifest.text.as_deref(),
            transcript
                .as_ref()
                .filter(|t| t.error.is_none())
                .map(|t| t.text.as_str()),
        )
        .is_some(),
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
        transcribing: inner.stt_is_running(id),
        waiting: None,
        deletable: true,
        speakable: read_aloud_text(
            n.manifest.text.as_deref(),
            transcript
                .filter(|t| t.error.is_none())
                .map(|t| t.text.as_str()),
        )
        .is_some(),
    }
}

impl Inner {
    fn note_info(&self, id: &NoteId) -> Option<NoteInfo> {
        if let Some(n) = self.from_phone.machine().note(id) {
            return Some(note_from_phone(self, n));
        }
        self.from_glass
            .machine()
            .note(id)
            .map(|n| note_from_glass(self, n))
    }

    /// Makes any `speak_note` so far not count, and removes its audio. Returns the generation
    /// for the next one.
    fn stop_speaking(&mut self) -> u64 {
        if let Some(path) = self.speech_file.take() {
            remove_file(&path);
        }
        self.speech_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn stt_is_running(&self, id: NoteId) -> bool {
        self.stt_running.iter().any(|(note, _)| *note == id)
    }

    fn stt_finished(&mut self, id: NoteId, attachment: u16) {
        self.stt_running.remove(&(id, attachment));
        self.stt_attempts.finished(id, attachment);
    }

    fn forget_stt_attempts(&self, id: NoteId) {
        self.stt_attempts.forget(id);
    }

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
        // What a note waits for is for this link to say again.
        self.deferred.clear();
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
                ReceiverEvent::NoteComplete { note_id } => {
                    self.deferred.remove(&note_id);
                    CoreEvent::NoteSynced {
                        note_id: note_id.to_hex(),
                    }
                }
                ReceiverEvent::NoteDeferred { note_id, reason } => {
                    let reason = defer_reason(&reason);
                    self.deferred.insert(note_id, reason.clone());
                    CoreEvent::NoteDeferred {
                        note_id: note_id.to_hex(),
                        reason,
                    }
                }
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
        self.report_progress(now_ms());
    }

    /// At most once per [`PROGRESS_INTERVAL_MS`], and only if the transfer moved.
    fn report_progress(&mut self, now: u64) {
        let Some(transfer) = self.from_glass.machine().transfer() else {
            self.last_progress = None;
            return;
        };
        if progress_due(self.last_progress, transfer, now) {
            self.last_progress = Some((transfer, now));
            self.events.push(CoreEvent::NoteProgress);
        }
    }

    fn finished(&mut self, f: Finished) {
        match f {
            Finished::Speech(outcome) => {
                if let SpeechOutcome::Transcribed {
                    note_id,
                    attachment_id,
                    ..
                } = &outcome
                {
                    self.stt_finished(*note_id, *attachment_id);
                }
                let frames = self.from_glass.complete_speech(outcome, now_ms());
                // Without a link the frames are dropped: the Glass asks again on the next one.
                if self.link.as_ref().is_some_and(|l| l.notes_sync) {
                    self.note_frames(frames);
                } else {
                    self.note_events();
                }
            }
            Finished::Spoken {
                request_id,
                generation,
                note_id,
                result,
            } => {
                if generation != self.speech_generation.load(Ordering::SeqCst) {
                    // Replaced or stopped while it ran.
                    if let Ok(path) = &result {
                        remove_file(path);
                    }
                    return;
                }
                let note_id = note_id.to_hex();
                self.events.push(match result {
                    Ok(path) => {
                        let shown = path.to_string_lossy().into_owned();
                        self.speech_file = Some(path);
                        CoreEvent::NoteSpeechReady {
                            request_id,
                            note_id,
                            path: shown,
                        }
                    }
                    Err(reason) => CoreEvent::NoteSpeechFailed {
                        request_id,
                        note_id,
                        reason,
                    },
                });
            }
            Finished::PhoneTranscript(note_id, attachment_id, result) => {
                self.stt_finished(note_id, attachment_id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_reported_at_the_start_then_only_when_it_moved_and_not_too_often() {
        let at = |received_bytes| CoreNoteTransfer {
            note_id: NoteId([1; 16]),
            received_bytes,
            total_bytes: 100,
        };
        assert!(progress_due(None, at(0), 5));
        assert!(!progress_due(Some((at(0), 5)), at(10), 5 + 999));
        assert!(progress_due(Some((at(0), 5)), at(10), 5 + 1000));
        assert!(!progress_due(Some((at(10), 5)), at(10), 5 + 5000));
        // A clock going backwards only delays a report.
        assert!(!progress_due(Some((at(0), 5)), at(10), 1));
    }
}
