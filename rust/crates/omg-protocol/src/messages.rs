//! The OpenMyGlass companion-link message schema, version 1.
//!
//! Every message on the wire is exactly one [`Frame`]. Variants are grouped by feature.
//! Backbone groups are **fully implemented** in `omg-core`; groups marked `SCAFFOLD`
//! are typed here (so glassd and the companion agree on the shape) but their handlers
//! are stubbed — they are the roadmap, not yet wired end-to-end.
//!
//! IMPORTANT for glassd: add new variants only at the END of [`Frame`] (postcard encodes
//! enums by discriminant).

use serde::{Deserialize, Serialize};

use crate::Locale;

/// Which side of the link a peer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Phone or PC running the OpenMyGlass companion.
    Companion,
    /// The Glass, running glassd.
    Glass,
}

/// One message on the wire. See the module docs for framing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Frame {
    // ===================== Session backbone (IMPLEMENTED) =====================
    /// First frame each direction. Carries identity + capabilities.
    Hello(Hello),
    /// Heartbeat request.
    Ping(Ping),
    /// Heartbeat reply (echoes the ping token).
    Pong(Pong),
    /// Graceful close with a reason.
    Bye(Bye),
    /// Application- or protocol-level error.
    Error(ErrorMsg),

    // ===================== Pairing & auth (IMPLEMENTED) =======================
    /// Companion -> Glass: request to become THE bonded companion (one-time pairing).
    PairRequest(PairRequest),
    /// Glass -> Companion: pairing accepted, here is my identity.
    PairAccept(PairAccept),
    /// Final pairing outcome (either direction).
    PairResult(PairResult),
    /// Forget the bonded peer (either direction). The next connection must re-pair.
    Unpair,
    /// Challenge the peer to prove ownership of its bonded identity key.
    AuthChallenge(AuthChallenge),
    /// Signature over the peer's challenge nonce.
    AuthResponse(AuthResponse),
    /// Result of verifying the peer's [`AuthResponse`].
    AuthResult(AuthResult),

    // ===================== Locale / i18n (IMPLEMENTED) ========================
    /// Companion -> Glass: set the UI locale.
    SetLocale(SetLocale),
    /// Glass -> Companion: the active locale (ack, or on change at setup).
    LocaleChanged(LocaleChanged),

    // ===================== Device status / battery (IMPLEMENTED) ==============
    /// Companion -> Glass: request current device status.
    StatusRequest(StatusRequest),
    /// Glass -> Companion: device status snapshot.
    Status(GlassStatus),
    /// Glass -> Companion: unsolicited battery push.
    BatteryStatus(BatteryStatus),

    // ===================== Notifications (IMPLEMENTED backbone) ===============
    /// Companion -> Glass: a new or updated phone notification.
    NotificationPosted(NotificationPosted),
    /// Companion -> Glass: a notification was dismissed anywhere.
    NotificationRemoved(NotificationRemoved),
    /// Glass -> Companion: the owner picked an action or dismissed the card.
    NotificationAction(NotificationAction),

    // ===================== Radio control + airplane (SCAFFOLD) ================
    /// Companion -> Glass: turn a single radio on/off (rfkill + supplicant / bluez).
    SetRadio(SetRadio),
    /// Companion -> Glass: master airplane toggle (all radios off).
    SetAirplaneMode(SetAirplaneMode),
    /// Glass -> Companion: current radio state (also embedded in [`GlassStatus`]).
    RadioState(RadioState),

    // ===================== On-demand Wi-Fi coordination (SCAFFOLD) ============
    /// Either side: "bring Wi-Fi up for a big transfer" (media sync / OTA / BT-less).
    WifiBringUpRequest(WifiBringUpRequest),
    /// Result of a [`WifiBringUpRequest`] (SSID/IP once associated).
    WifiBringUpResult(WifiBringUpResult),
    /// Either side: transfer done, drop Wi-Fi again (battery).
    WifiTearDown,

    // ===================== Time sync (SCAFFOLD) ===============================
    /// Companion -> Glass: wall clock + timezone.
    TimeSync(TimeSync),

    // ===================== Location + navigation (SCAFFOLD) ===================
    /// Glass -> Companion: start/change/stop phone-GPS location updates.
    LocationSubscribe(LocationSubscribe),
    /// Companion -> Glass: a location fix while subscribed.
    LocationUpdate(LocationUpdate),
    /// Companion -> Glass: cannot serve the subscription (and why).
    LocationUnavailable(LocationUnavailable),
    /// Companion -> Glass: current turn-by-turn maneuver card.
    NavState(NavState),
    /// Companion -> Glass: navigation ended, remove the card.
    NavEnded,

    // ===================== Dictation relay (SCAFFOLD) =========================
    /// Glass -> Companion: push-to-talk started (reply or free dictation).
    DictationStart(DictationStart),
    /// Glass -> Companion: a chunk of mic PCM.
    AudioChunk(AudioChunk),
    /// Glass -> Companion: speech ended (or cancelled).
    DictationEnd(DictationEnd),
    /// Companion -> Glass: the offline/STT transcript (or why there is none).
    Transcript(Transcript),
    /// Glass -> Companion: send or discard the transcript.
    ReplyConfirm(ReplyConfirm),
    /// Companion -> Glass: whether the reply went out.
    ReplyResult(ReplyResult),

    // ===================== Media sync (SCAFFOLD) ==============================
    /// Companion -> Glass: list media on the Glass.
    MediaListRequest,
    /// Glass -> Companion: the media list.
    MediaList(MediaList),
    /// Companion -> Glass: open the media server for one Wi-Fi pull.
    MediaPullStart,
    /// Glass -> Companion: where to download (token valid for this pull).
    MediaServer(MediaServer),
    /// Glass -> Companion: no Wi-Fi media server available (and why).
    MediaServerUnavailable(MediaServerUnavailable),
    /// Companion -> Glass: close the media server.
    MediaPullDone,
    /// Companion -> Glass: Bluetooth fallback pull, one chunk at a time.
    MediaChunkRequest(MediaChunkRequest),
    /// Glass -> Companion: a media chunk.
    MediaChunk(MediaChunk),
    /// Companion -> Glass: saved+verified, delete the original (owner setting).
    MediaDelete(MediaDelete),
    /// Glass -> Companion: delete result.
    MediaDeleted(MediaDeleted),

    // ===================== Live captions / translation (SCAFFOLD) ============
    /// Glass -> Companion: start live captions/translation (mic -> companion STT/MT).
    CaptionStart(CaptionStart),
    /// Glass -> Companion: caption audio chunk (same shape as dictation).
    CaptionAudioChunk(AudioChunk),
    /// Companion -> Glass: a caption line (and its translation).
    CaptionText(CaptionText),
    /// Either side: captions ended.
    CaptionEnd(CaptionEnd),

    // ===================== Assistant, push-to-talk (SCAFFOLD) ================
    /// Glass -> Companion: assistant push-to-talk started.
    AssistantStart(AssistantStart),
    /// Glass -> Companion: assistant audio chunk.
    AssistantAudioChunk(AudioChunk),
    /// Glass -> Companion: assistant speech ended.
    AssistantEnd(AssistantEnd),
    /// Companion -> Glass: assistant answer + optional action.
    AssistantResult(AssistantResult),

    // ===================== Add-ons / Glassware (SCAFFOLD) ====================
    /// Companion -> Glass: list installed add-ons.
    AddonListRequest,
    /// Glass -> Companion: installed add-ons.
    AddonList(AddonList),
    /// Companion -> Glass: install/sideload an add-on.
    AddonInstall(AddonInstall),
    /// Glass -> Companion: install progress/result.
    AddonProgress(AddonProgress),
    /// Companion -> Glass: uninstall an add-on.
    AddonUninstall(AddonUninstall),

    // ===================== Settings mirror (SCAFFOLD) ========================
    /// Companion -> Glass: read current settings.
    SettingsGet,
    /// Glass -> Companion: current settings.
    Settings(Settings),
    /// Companion -> Glass: change settings (only `Some` fields are applied).
    SettingsSet(Settings),

    // ===================== Screencast (SCAFFOLD) =============================
    /// Companion -> Glass: start mirroring the prism.
    ScreencastStart,
    /// Companion -> Glass: stop mirroring.
    ScreencastStop,
    /// Glass -> Companion: one JPEG frame of the prism.
    ScreenFrame(ScreenFrame),

    // ===================== Security: find / lock / wipe (SCAFFOLD) ===========
    /// Companion -> Glass: make a sound / show a locator card.
    RemoteFind,
    /// Companion -> Glass: lock the device.
    RemoteLock,
    /// Companion -> Glass: factory wipe (requires a confirm token).
    RemoteWipe(RemoteWipe),

    // ===================== OTA updates (SCAFFOLD) ============================
    /// Companion -> Glass: an update is available.
    OtaOffer(OtaOffer),
    /// Companion -> Glass: begin transfer.
    OtaStart(OtaStart),
    /// Companion -> Glass: an OTA data chunk.
    OtaChunk(OtaChunk),
    /// Either side: OTA finished.
    OtaDone(OtaDone),

    // ===================== Tethering (SCAFFOLD) =============================
    /// Glass -> Companion: request/stop internet via the companion (BT-PAN / Wi-Fi).
    TetherRequest(TetherRequest),
    /// Companion -> Glass: tethering state.
    TetherState(TetherState),

    // ===================== Notes with attachments (IMPLEMENTED in omg-core) ====
    // Send only to a peer that advertised `capability::NOTES_SYNC`: an older peer treats
    // an unknown variant as a fatal decode error. See `crate::notes`.
    /// Glass -> Companion: a note is waiting.
    NoteOffer(crate::notes::NoteOffer),
    /// Companion -> Glass: accepted with resume offsets, already held, deferred or refused.
    NoteOfferReply(crate::notes::NoteOfferReply),
    /// Companion -> Glass: send these bytes of an attachment.
    NoteChunkRequest(crate::notes::NoteChunkRequest),
    /// Glass -> Companion: bytes of an attachment, with their SHA-256.
    NoteChunk(crate::notes::NoteChunk),
    /// Companion -> Glass: a chunk was discarded and will be requested again.
    NoteChunkReject(crate::notes::NoteChunkReject),
    /// Glass -> Companion: a chunk request cannot be served.
    NoteChunkUnavailable(crate::notes::NoteChunkUnavailable),
    /// Companion -> Glass: verdict on a whole attachment.
    NoteAttachmentAck(crate::notes::NoteAttachmentAck),
    /// Companion -> Glass: the whole note is durable and verified.
    NoteAck(crate::notes::NoteAck),
    /// Glass -> Companion: what is still waiting.
    NoteSyncStatus(crate::notes::NoteSyncStatus),
    /// Glass -> Companion: transcribe this note's voice recording.
    NoteTranscriptRequest(crate::notes::NoteTranscriptRequest),
    /// Companion -> Glass: the transcript, or why there is none.
    NoteTranscript(crate::notes::NoteTranscript),
    /// Glass -> Companion: transcript received.
    NoteTranscriptAck(crate::notes::NoteTranscriptAck),
    /// Glass -> Companion: speak this, for this note.
    NoteTtsRequest(crate::notes::NoteTtsRequest),
    /// Companion -> Glass: synthesized audio is ready to pull.
    NoteTtsReady(crate::notes::NoteTtsReady),
    /// Companion -> Glass: no audio for this request.
    NoteTtsFailed(crate::notes::NoteTtsFailed),
    /// Glass -> Companion: send these audio bytes.
    NoteTtsChunkRequest(crate::notes::NoteTtsChunkRequest),
    /// Companion -> Glass: audio bytes, with their SHA-256.
    NoteTtsChunk(crate::notes::NoteTtsChunk),
    /// Glass -> Companion: finished with this audio.
    NoteTtsRelease(crate::notes::NoteTtsRelease),
}

impl Frame {
    /// A short name for logging/metrics (no payload).
    pub fn wire_name(&self) -> &'static str {
        match self {
            Frame::Hello(_) => "Hello",
            Frame::Ping(_) => "Ping",
            Frame::Pong(_) => "Pong",
            Frame::Bye(_) => "Bye",
            Frame::Error(_) => "Error",
            Frame::PairRequest(_) => "PairRequest",
            Frame::PairAccept(_) => "PairAccept",
            Frame::PairResult(_) => "PairResult",
            Frame::Unpair => "Unpair",
            Frame::AuthChallenge(_) => "AuthChallenge",
            Frame::AuthResponse(_) => "AuthResponse",
            Frame::AuthResult(_) => "AuthResult",
            Frame::SetLocale(_) => "SetLocale",
            Frame::LocaleChanged(_) => "LocaleChanged",
            Frame::StatusRequest(_) => "StatusRequest",
            Frame::Status(_) => "Status",
            Frame::BatteryStatus(_) => "BatteryStatus",
            Frame::NotificationPosted(_) => "NotificationPosted",
            Frame::NotificationRemoved(_) => "NotificationRemoved",
            Frame::NotificationAction(_) => "NotificationAction",
            Frame::SetRadio(_) => "SetRadio",
            Frame::SetAirplaneMode(_) => "SetAirplaneMode",
            Frame::RadioState(_) => "RadioState",
            Frame::WifiBringUpRequest(_) => "WifiBringUpRequest",
            Frame::WifiBringUpResult(_) => "WifiBringUpResult",
            Frame::WifiTearDown => "WifiTearDown",
            Frame::TimeSync(_) => "TimeSync",
            Frame::LocationSubscribe(_) => "LocationSubscribe",
            Frame::LocationUpdate(_) => "LocationUpdate",
            Frame::LocationUnavailable(_) => "LocationUnavailable",
            Frame::NavState(_) => "NavState",
            Frame::NavEnded => "NavEnded",
            Frame::DictationStart(_) => "DictationStart",
            Frame::AudioChunk(_) => "AudioChunk",
            Frame::DictationEnd(_) => "DictationEnd",
            Frame::Transcript(_) => "Transcript",
            Frame::ReplyConfirm(_) => "ReplyConfirm",
            Frame::ReplyResult(_) => "ReplyResult",
            Frame::MediaListRequest => "MediaListRequest",
            Frame::MediaList(_) => "MediaList",
            Frame::MediaPullStart => "MediaPullStart",
            Frame::MediaServer(_) => "MediaServer",
            Frame::MediaServerUnavailable(_) => "MediaServerUnavailable",
            Frame::MediaPullDone => "MediaPullDone",
            Frame::MediaChunkRequest(_) => "MediaChunkRequest",
            Frame::MediaChunk(_) => "MediaChunk",
            Frame::MediaDelete(_) => "MediaDelete",
            Frame::MediaDeleted(_) => "MediaDeleted",
            Frame::CaptionStart(_) => "CaptionStart",
            Frame::CaptionAudioChunk(_) => "CaptionAudioChunk",
            Frame::CaptionText(_) => "CaptionText",
            Frame::CaptionEnd(_) => "CaptionEnd",
            Frame::AssistantStart(_) => "AssistantStart",
            Frame::AssistantAudioChunk(_) => "AssistantAudioChunk",
            Frame::AssistantEnd(_) => "AssistantEnd",
            Frame::AssistantResult(_) => "AssistantResult",
            Frame::AddonListRequest => "AddonListRequest",
            Frame::AddonList(_) => "AddonList",
            Frame::AddonInstall(_) => "AddonInstall",
            Frame::AddonProgress(_) => "AddonProgress",
            Frame::AddonUninstall(_) => "AddonUninstall",
            Frame::SettingsGet => "SettingsGet",
            Frame::Settings(_) => "Settings",
            Frame::SettingsSet(_) => "SettingsSet",
            Frame::ScreencastStart => "ScreencastStart",
            Frame::ScreencastStop => "ScreencastStop",
            Frame::ScreenFrame(_) => "ScreenFrame",
            Frame::RemoteFind => "RemoteFind",
            Frame::RemoteLock => "RemoteLock",
            Frame::RemoteWipe(_) => "RemoteWipe",
            Frame::OtaOffer(_) => "OtaOffer",
            Frame::OtaStart(_) => "OtaStart",
            Frame::OtaChunk(_) => "OtaChunk",
            Frame::OtaDone(_) => "OtaDone",
            Frame::TetherRequest(_) => "TetherRequest",
            Frame::TetherState(_) => "TetherState",
            Frame::NoteOffer(_) => "NoteOffer",
            Frame::NoteOfferReply(_) => "NoteOfferReply",
            Frame::NoteChunkRequest(_) => "NoteChunkRequest",
            Frame::NoteChunk(_) => "NoteChunk",
            Frame::NoteChunkReject(_) => "NoteChunkReject",
            Frame::NoteChunkUnavailable(_) => "NoteChunkUnavailable",
            Frame::NoteAttachmentAck(_) => "NoteAttachmentAck",
            Frame::NoteAck(_) => "NoteAck",
            Frame::NoteSyncStatus(_) => "NoteSyncStatus",
            Frame::NoteTranscriptRequest(_) => "NoteTranscriptRequest",
            Frame::NoteTranscript(_) => "NoteTranscript",
            Frame::NoteTranscriptAck(_) => "NoteTranscriptAck",
            Frame::NoteTtsRequest(_) => "NoteTtsRequest",
            Frame::NoteTtsReady(_) => "NoteTtsReady",
            Frame::NoteTtsFailed(_) => "NoteTtsFailed",
            Frame::NoteTtsChunkRequest(_) => "NoteTtsChunkRequest",
            Frame::NoteTtsChunk(_) => "NoteTtsChunk",
            Frame::NoteTtsRelease(_) => "NoteTtsRelease",
        }
    }
}

// --------------------------- Session backbone ---------------------------

/// First frame each direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    /// Sender's [`crate::PROTOCOL_VERSION`].
    pub protocol_version: u16,
    /// Which side the sender is.
    pub role: Role,
    /// Human-readable version of the companion/glassd build.
    pub app_version: String,
    /// Friendly device name (shown in the pairing UI).
    pub device_name: String,
    /// base64 of the sender's 32-byte ed25519 identity public key (the trust anchor).
    pub identity_pub: String,
    /// Capability strings (see [`crate::capability`]).
    pub capabilities: Vec<String>,
}

/// Heartbeat request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ping {
    /// Opaque token echoed back in [`Pong`] to measure round-trip time.
    pub token: u64,
    /// Sender wall-clock, milliseconds since the Unix epoch.
    pub unix_ms: u64,
}

/// Heartbeat reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pong {
    /// The token from the matching [`Ping`].
    pub token: u64,
    /// Replier wall-clock, milliseconds since the Unix epoch.
    pub unix_ms: u64,
}

/// Graceful close.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bye {
    /// Why the sender is closing.
    pub reason: String,
}

/// Application- or protocol-level error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorMsg {
    /// Machine-readable code.
    pub code: ErrorCode,
    /// Human-readable detail.
    pub message: String,
}

/// Error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    /// Peer identity does not match the bonded companion/glass.
    UnknownPeer,
    /// Challenge/response signature did not verify.
    AuthFailed,
    /// No pairing exists and the peer did not offer one.
    NotPaired,
    /// A pairing already exists; refusing to bond a second companion.
    AlreadyPaired,
    /// Protocol versions are incompatible.
    VersionMismatch,
    /// Malformed or out-of-sequence frame.
    Protocol,
    /// Feature not implemented on this build.
    Unsupported,
    /// Anything else.
    Internal,
}

// --------------------------- Pairing & auth ---------------------------

/// Companion -> Glass: request one-time pairing as THE bonded companion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairRequest {
    /// Friendly name of the companion.
    pub companion_name: String,
    /// base64 of the companion's 32-byte ed25519 public key.
    pub companion_pub: String,
    /// base64 of a fresh 16-byte random nonce (replay protection).
    pub nonce: String,
    /// Optional one-time bootstrap token from the Wi-Fi QR, binding this first
    /// pairing to the companion that generated the QR.
    pub bootstrap_token: Option<String>,
}

/// Glass -> Companion: pairing accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairAccept {
    /// Friendly name of the Glass.
    pub glass_name: String,
    /// base64 of the Glass's 32-byte ed25519 public key.
    pub glass_pub: String,
    /// Echo of the companion's nonce.
    pub nonce: String,
}

/// Terminal pairing outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairResult {
    /// Whether pairing succeeded.
    pub ok: bool,
    /// Human-readable detail.
    pub message: String,
}

/// Challenge the peer to sign a nonce with its bonded identity key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthChallenge {
    /// base64 of a fresh 32-byte random nonce.
    pub nonce: String,
}

/// Signature over `b"openmyglass-companion-auth-v1" || nonce`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthResponse {
    /// base64 of the 64-byte ed25519 signature.
    pub signature: String,
}

/// Result of verifying the peer's [`AuthResponse`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthResult {
    /// Whether the peer's signature verified against the bonded key.
    pub ok: bool,
    /// Human-readable detail.
    pub message: String,
}

// --------------------------- Locale ---------------------------

/// Companion -> Glass: set the UI locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetLocale {
    /// The locale to activate.
    pub locale: Locale,
}

/// Glass -> Companion: the active locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocaleChanged {
    /// The now-active locale.
    pub locale: Locale,
}

// --------------------------- Status / battery ---------------------------

/// Companion -> Glass: request a status snapshot. All-false means "everything".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusRequest {
    /// Battery percentage + charging state.
    pub battery: bool,
    /// Internal storage totals.
    pub storage: bool,
    /// Device name / software version / screen size.
    pub device: bool,
    /// Radio state.
    pub radios: bool,
}

/// Glass -> Companion: device status. Absent (`None`) fields are unknown/not requested.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GlassStatus {
    /// Battery charge, 0-100.
    pub battery_percent: Option<u8>,
    /// Whether the Glass is charging.
    pub charging: Option<bool>,
    /// Internal storage total, bytes.
    pub storage_total_bytes: Option<u64>,
    /// Internal storage free, bytes.
    pub storage_free_bytes: Option<u64>,
    /// Bluetooth device name.
    pub device_name: Option<String>,
    /// glassd / ROM version string.
    pub software_version: Option<String>,
    /// Prism width in pixels (640 on XE24).
    pub screen_width_px: Option<u32>,
    /// Prism height in pixels (360 on XE24).
    pub screen_height_px: Option<u32>,
    /// Battery/skin temperature, Celsius.
    pub temperature_c: Option<f32>,
    /// Current radio state.
    pub radios: Option<RadioState>,
    /// Active UI locale.
    pub locale: Option<Locale>,
}

/// Glass -> Companion: unsolicited battery push (on change).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatteryStatus {
    /// Battery charge, 0-100.
    pub percent: u8,
    /// Whether the Glass is charging.
    pub charging: bool,
}

// --------------------------- Notifications ---------------------------

/// Companion -> Glass: a new or updated phone notification (already filtered/truncated).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationPosted {
    /// The notification payload.
    pub notification: Notification,
}

/// A phone notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    /// Stable key identifying the notification across updates/actions.
    pub key: String,
    /// App display name.
    pub app_name: String,
    /// Title (truncated by the companion).
    pub title: String,
    /// Body text (truncated by the companion).
    pub text: String,
    /// When the notification was posted, Unix ms.
    pub posted_at_ms: u64,
    /// SHA-256 of the app icon (sent separately), if any.
    pub icon_sha256: Option<Vec<u8>>,
    /// Available actions (reply / buttons).
    pub actions: Vec<NotificationActionInfo>,
    /// Whether the owner allowed this notification to wake the prism.
    pub wake: bool,
}

/// One action attached to a notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationActionInfo {
    /// Index of the action on the phone.
    pub index: u32,
    /// Button label.
    pub title: String,
    /// Whether choosing it opens a dictated reply (has a RemoteInput).
    pub reply: bool,
}

/// Companion -> Glass: a notification was dismissed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationRemoved {
    /// Key of the removed notification.
    pub key: String,
}

/// Glass -> Companion: the owner chose an action or dismissed the card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationAction {
    /// Key of the notification acted on.
    pub key: String,
    /// What the owner chose.
    pub choice: NotificationChoice,
}

/// What the owner did with a notification card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationChoice {
    /// Invoke the action at this index.
    Action(u32),
    /// Dismiss the notification.
    Dismiss,
}

// --------------------------- Radio control + airplane ---------------------------

/// A controllable radio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Radio {
    /// Wi-Fi (BCM4330, rfkill + wpa_supplicant + udhcpc).
    Wifi,
    /// Bluetooth (bcm4330.hcd over HCI UART, bluez).
    Bt,
}

/// Companion -> Glass: toggle one radio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetRadio {
    /// Which radio.
    pub radio: Radio,
    /// Desired state.
    pub on: bool,
}

/// Companion -> Glass: master airplane toggle (all radios off at once via rfkill).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetAirplaneMode {
    /// Desired state.
    pub on: bool,
}

/// Current radio state.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RadioState {
    /// Whether Wi-Fi is on.
    pub wifi_on: bool,
    /// Whether Bluetooth is on.
    pub bt_on: bool,
    /// Whether airplane mode (all radios off) is engaged.
    pub airplane_mode: bool,
    /// Associated SSID, when Wi-Fi is up.
    pub wifi_ssid: Option<String>,
    /// Wi-Fi IP address, when up.
    pub wifi_ip: Option<String>,
}

// --------------------------- On-demand Wi-Fi ---------------------------

/// Why Wi-Fi needs to come up (it stays off by default — battery).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WifiReason {
    /// Photo/video backup.
    MediaSync,
    /// ROM/app OTA.
    Ota,
    /// No Bluetooth available (e.g. a PC companion without BT).
    Fallback,
    /// Anything else.
    Other,
}

/// Either side: request Wi-Fi be brought up for a bandwidth-heavy job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiBringUpRequest {
    /// Why.
    pub reason: WifiReason,
}

/// Result of a [`WifiBringUpRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiBringUpResult {
    /// Whether Wi-Fi came up.
    pub ok: bool,
    /// SSID once associated.
    pub ssid: Option<String>,
    /// IP once associated.
    pub ip: Option<String>,
    /// Detail on failure.
    pub message: String,
}

// --------------------------- Time sync ---------------------------

/// Companion -> Glass: set wall clock + timezone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeSync {
    /// Milliseconds since the Unix epoch.
    pub unix_ms: u64,
    /// IANA timezone name, e.g. `Europe/Rome`.
    pub tz: String,
}

// --------------------------- Location + navigation ---------------------------

/// Glass -> Companion: subscribe to phone-GPS location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationSubscribe {
    /// Desired update period, ms. 0 stops updates.
    pub interval_ms: u32,
    /// GPS-grade (more battery) vs balanced (network/fused).
    pub high_accuracy: bool,
}

/// Companion -> Glass: a location fix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocationUpdate {
    /// WGS-84 latitude, degrees.
    pub latitude_deg: f64,
    /// WGS-84 longitude, degrees.
    pub longitude_deg: f64,
    /// Horizontal accuracy (68% radius), metres. 0 = unknown.
    pub accuracy_m: f32,
    /// Altitude above the WGS-84 ellipsoid, metres.
    pub altitude_m: Option<f64>,
    /// Bearing, degrees clockwise from true north.
    pub bearing_deg: Option<f32>,
    /// Ground speed, m/s.
    pub speed_mps: Option<f32>,
    /// Fix time, Unix ms.
    pub fix_time_ms: u64,
    /// `gps`, `network`, or `fused`.
    pub provider: String,
}

/// Companion -> Glass: subscription cannot be served right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationUnavailable {
    /// `permission`, `disabled`, or `no-fix`.
    pub reason: String,
}

/// A navigation maneuver icon. Numbering matches the existing `bridge.proto` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Maneuver {
    /// Unknown / not yet computed.
    Unknown,
    Straight,
    Left,
    SlightLeft,
    SharpLeft,
    Right,
    SlightRight,
    SharpRight,
    KeepLeft,
    KeepRight,
    UTurnLeft,
    UTurnRight,
    OffRoute,
    Roundabout,
    RoundaboutLeft,
    Arrive,
}

/// Companion -> Glass: the current turn-by-turn card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavState {
    /// Next maneuver.
    pub maneuver: Maneuver,
    /// Distance to the maneuver, metres.
    pub distance_m: Option<u32>,
    /// Spoken-style instruction.
    pub instruction: Option<String>,
    /// Street / road name.
    pub street: Option<String>,
    /// Estimated time of arrival text.
    pub eta: Option<String>,
    /// Route source, e.g. `osmand` or `gmaps`.
    pub source: String,
    /// SHA-256 of the maneuver icon (sent separately).
    pub icon_sha256: Option<Vec<u8>>,
}

// --------------------------- Dictation ---------------------------

/// Glass -> Companion: push-to-talk started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictationStart {
    /// Correlates audio + transcript + confirm.
    pub request_id: u32,
    /// Notification key for a reply (ignored when free).
    pub key: Option<String>,
    /// Reply action index on the phone (ignored when free).
    pub action_index: Option<u32>,
    /// PCM sample rate (e.g. 16000).
    pub sample_rate: u32,
    /// Free dictation (not a notification reply).
    pub free: bool,
}

/// Mic audio. PCM 16-bit LE mono at the start's `sample_rate`, ~100 ms per chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioChunk {
    /// Matches [`DictationStart::request_id`] / caption / assistant request.
    pub request_id: u32,
    /// Monotonic chunk sequence.
    pub seq: u32,
    /// Raw PCM bytes.
    pub pcm: Vec<u8>,
}

/// Glass -> Companion: speech ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictationEnd {
    /// Request id.
    pub request_id: u32,
    /// Whether the owner cancelled.
    pub cancelled: bool,
}

/// Companion -> Glass: the transcript (or why there is none).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    /// Request id.
    pub request_id: u32,
    /// Transcribed text.
    pub text: String,
    /// Whether this is the final transcript.
    #[serde(rename = "final")]
    pub is_final: bool,
    /// Non-empty when there is no usable transcript.
    pub error: Option<String>,
}

/// Glass -> Companion: send or discard the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyConfirm {
    /// Request id.
    pub request_id: u32,
    /// Send (true) or discard (false).
    pub send: bool,
}

/// Companion -> Glass: whether the reply went out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyResult {
    /// Request id.
    pub request_id: u32,
    /// Whether it was sent.
    pub ok: bool,
    /// Detail.
    pub message: String,
}

// --------------------------- Media sync ---------------------------

/// One media item on the Glass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaItem {
    /// Stable id (path+size+mtime).
    pub id: String,
    /// File name.
    pub name: String,
    /// Whether it is a video.
    pub video: bool,
    /// Size, bytes.
    pub size: u64,
    /// Capture time, Unix ms.
    pub taken_at_ms: u64,
}

/// Glass -> Companion: the media list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaList {
    /// Items.
    pub items: Vec<MediaItem>,
}

/// Glass -> Companion: Wi-Fi media server coordinates for one pull.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaServer {
    /// Server IP.
    pub ip: String,
    /// Server port.
    pub port: u16,
    /// One-pull token (GET /media/<id>?token=…; Range supported).
    pub token: String,
}

/// Glass -> Companion: no Wi-Fi media server available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaServerUnavailable {
    /// Why.
    pub reason: String,
}

/// Companion -> Glass: Bluetooth fallback pull request (<= 48 KiB/chunk).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaChunkRequest {
    /// Media id.
    pub id: String,
    /// Byte offset.
    pub offset: u64,
    /// Requested length.
    pub length: u32,
}

/// Glass -> Companion: a media chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaChunk {
    /// Media id.
    pub id: String,
    /// Byte offset.
    pub offset: u64,
    /// Data (empty at EOF or on error).
    pub data: Vec<u8>,
    /// Total file size.
    pub total: u64,
    /// SHA-256 of the whole file (only in the final chunk).
    pub sha256: Option<String>,
    /// Error detail, if any.
    pub error: Option<String>,
}

/// Companion -> Glass: delete a media item (saved+verified).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaDelete {
    /// Media id.
    pub id: String,
}

/// Glass -> Companion: delete result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaDeleted {
    /// Media id.
    pub id: String,
    /// Whether it was deleted.
    pub ok: bool,
}

// --------------------------- Captions / translation ---------------------------

/// Glass -> Companion: start live captions / translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionStart {
    /// Request id.
    pub request_id: u32,
    /// Spoken-language locale.
    pub source_lang: Locale,
    /// Target locale for translation (None = captions only).
    pub target_lang: Option<Locale>,
    /// PCM sample rate.
    pub sample_rate: u32,
}

/// Companion -> Glass: one caption line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionText {
    /// Request id.
    pub request_id: u32,
    /// Recognised text (source language).
    pub text: String,
    /// Translated text, if a target was set.
    pub translated: Option<String>,
    /// Whether this line is final (vs interim).
    #[serde(rename = "final")]
    pub is_final: bool,
}

/// Either side: captions ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionEnd {
    /// Request id.
    pub request_id: u32,
}

// --------------------------- Assistant ---------------------------

/// Glass -> Companion: assistant push-to-talk started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantStart {
    /// Request id.
    pub request_id: u32,
    /// PCM sample rate.
    pub sample_rate: u32,
}

/// Glass -> Companion: assistant speech ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantEnd {
    /// Request id.
    pub request_id: u32,
    /// Whether cancelled.
    pub cancelled: bool,
}

/// Companion -> Glass: assistant answer + optional action to run on the companion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantResult {
    /// Request id.
    pub request_id: u32,
    /// Answer text to show on the prism.
    pub text: String,
    /// Structured action for the companion to execute, if any.
    pub action: Option<AssistantAction>,
    /// Error detail, if any.
    pub error: Option<String>,
}

/// An action the assistant resolved to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssistantAction {
    /// Start navigation to a destination.
    Navigate {
        /// Free-text destination.
        destination: String,
    },
    /// Send a message.
    SendMessage {
        /// Recipient.
        to: String,
        /// Body.
        body: String,
    },
    /// Web/search query (answered by the companion gateway).
    Search {
        /// Query text.
        query: String,
    },
    /// Open a URL on the companion.
    OpenUrl {
        /// URL.
        url: String,
    },
}

// --------------------------- Add-ons ---------------------------

/// One installed add-on (Glassware).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddonInfo {
    /// Add-on id (package/bundle).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Version string.
    pub version: String,
    /// Whether it is enabled.
    pub enabled: bool,
}

/// Glass -> Companion: installed add-ons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddonList {
    /// Add-ons.
    pub addons: Vec<AddonInfo>,
}

/// Companion -> Glass: install/sideload an add-on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddonInstall {
    /// Add-on id.
    pub id: String,
    /// Source URL (companion fetches and streams it), if sideloading.
    pub source_url: Option<String>,
}

/// Glass -> Companion: install progress/result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddonProgress {
    /// Add-on id.
    pub id: String,
    /// 0-100.
    pub percent: u8,
    /// Free-text state (`downloading`, `installing`, `done`, `failed`).
    pub state: String,
    /// Error detail, if any.
    pub error: Option<String>,
}

/// Companion -> Glass: uninstall an add-on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddonUninstall {
    /// Add-on id.
    pub id: String,
}

// --------------------------- Settings mirror ---------------------------

/// Glass settings: every setting can be read and changed from the companion. `Some` fields are
/// read/applied; `None` fields are untouched.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// Prism brightness, 0-100.
    pub brightness: Option<u8>,
    /// Wake-on-head-up.
    pub head_wake: Option<bool>,
    /// Wake-on-head-up angle, degrees.
    pub head_wake_angle_deg: Option<u8>,
    /// On-head detection.
    pub on_head_detection: Option<bool>,
    /// Earpiece volume, 0-100 percent.
    pub audio_volume: Option<u8>,
    /// 24-hour clock, else 12-hour.
    pub time_24h: Option<bool>,
    /// Index into the Glass's time zone table.
    pub timezone_id: Option<u32>,
    /// Energy policy.
    pub power_saver: Option<bool>,
    /// Suspend to RAM when idle.
    pub deep_sleep: Option<bool>,
    /// Idle before suspend, seconds.
    pub deep_sleep_idle_s: Option<u32>,
    /// Wink-to-capture.
    pub wink: Option<bool>,
    /// Screen timeout, seconds.
    pub screen_timeout_s: Option<u32>,
    /// Head-nudge-to-wake.
    pub nudge: Option<bool>,
    /// UI locale.
    pub locale: Option<Locale>,
    /// Radio state (read-only here; use [`SetRadio`]/[`SetAirplaneMode`] to change).
    pub radios: Option<RadioState>,
    /// Minutes off the head before the Glass powers off: 0 = never, at most 720.
    #[serde(default, deserialize_with = "absent_at_the_end")]
    pub off_head_power_off_min: Option<u32>,
}

/// Decodes a field added at the end of [`Settings`], which this struct carried on the wire
/// before the field existed. postcard is positional, but a frame is length-delimited and the
/// struct ends it, so the two ages still understand each other: an older peer ignores the
/// extra bytes, and when an older peer's frame ends here the field is `None`. Later fields
/// must go after this one and decode the same way.
fn absent_at_the_end<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    Ok(Option::<u32>::deserialize(d).unwrap_or(None))
}

impl Settings {
    /// Copies the `Some` fields of `change` over this one. `radios` is read-only here.
    pub fn merge(&mut self, change: &Settings) {
        macro_rules! take {
            ($($f:ident),*) => { $( if change.$f.is_some() { self.$f = change.$f.clone(); } )* };
        }
        take!(
            brightness,
            head_wake,
            head_wake_angle_deg,
            on_head_detection,
            audio_volume,
            time_24h,
            timezone_id,
            power_saver,
            deep_sleep,
            deep_sleep_idle_s,
            wink,
            screen_timeout_s,
            nudge,
            locale,
            off_head_power_off_min
        );
    }
}

/// Names of the time zones [`Settings::timezone_id`] indexes. The same order as the table
/// glassd reads the rules from, which only ever grows at the end.
pub const TIME_ZONE_NAMES: &[&str] = &[
    "UTC",
    "London",
    "Roma, Paris, Madrid",
    "Athina, Helsinki",
    "Moskva",
    "Dubai",
    "Delhi",
    "Beijing, Singapore",
    "Tokyo",
    "Sydney",
    "New York",
    "Chicago",
    "Denver",
    "Los Angeles",
    "São Paulo",
];

// --------------------------- Screencast ---------------------------

/// Glass -> Companion: one JPEG frame of the prism (640x360 on XE24).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenFrame {
    /// JPEG bytes.
    pub jpeg: Vec<u8>,
    /// Frame width.
    pub width: u32,
    /// Frame height.
    pub height: u32,
    /// Whether the prism was on.
    pub screen_on: bool,
}

// --------------------------- Security ---------------------------

/// Companion -> Glass: factory wipe (destructive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWipe {
    /// A token the owner must confirm, to prevent accidental wipes.
    pub confirm_token: String,
}

// --------------------------- OTA ---------------------------

/// What an OTA payload updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OtaKind {
    /// Full ROM / system image.
    Rom,
    /// The companion-facing glassd app.
    App,
    /// An add-on.
    Addon,
}

/// Companion -> Glass: an update is available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtaOffer {
    /// Version string.
    pub version: String,
    /// Payload size, bytes.
    pub size_bytes: u64,
    /// Hex SHA-256 of the payload.
    pub sha256: String,
    /// What it updates.
    pub kind: OtaKind,
}

/// Companion -> Glass: begin transfer of a previously offered update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtaStart {
    /// Version being sent.
    pub version: String,
}

/// Companion -> Glass: a chunk of the OTA payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtaChunk {
    /// Byte offset.
    pub offset: u64,
    /// Data.
    pub data: Vec<u8>,
}

/// Either side: OTA finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtaDone {
    /// Whether it succeeded.
    pub ok: bool,
    /// Detail.
    pub message: String,
}

// --------------------------- Tethering ---------------------------

/// Glass -> Companion: request/stop internet via the companion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TetherRequest {
    /// Start (true) or stop (false).
    pub on: bool,
}

/// Companion -> Glass: tethering state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TetherState {
    /// Whether tethering is active.
    pub on: bool,
    /// Transport kind (`bt-pan`, `wifi`).
    pub kind: String,
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn merge_copies_only_the_fields_present_and_leaves_radios_alone() {
        let mut s = Settings {
            audio_volume: Some(87),
            time_24h: Some(true),
            radios: None,
            ..Default::default()
        };
        s.merge(&Settings {
            audio_volume: Some(40),
            radios: Some(RadioState {
                wifi_on: true,
                bt_on: true,
                airplane_mode: false,
                wifi_ssid: None,
                wifi_ip: None,
            }),
            ..Default::default()
        });
        assert_eq!(s.audio_volume, Some(40));
        assert_eq!(s.time_24h, Some(true));
        assert_eq!(s.radios, None);
    }

    #[test]
    fn merge_copies_the_power_off_time_even_when_it_is_never() {
        let mut s = Settings {
            off_head_power_off_min: Some(30),
            ..Default::default()
        };
        s.merge(&Settings::default());
        assert_eq!(s.off_head_power_off_min, Some(30));
        s.merge(&Settings {
            off_head_power_off_min: Some(0),
            ..Default::default()
        });
        assert_eq!(s.off_head_power_off_min, Some(0));
    }

    /// The frame an older peer sends: the same bytes without the last field. `None` is one
    /// byte in postcard.
    fn from_an_older_peer(s: &Settings) -> Vec<u8> {
        assert_eq!(s.off_head_power_off_min, None);
        let mut bytes = postcard::to_allocvec(&Frame::Settings(s.clone())).unwrap();
        assert_eq!(bytes.pop(), Some(0));
        bytes
    }

    #[test]
    fn a_settings_frame_from_an_older_peer_still_decodes() {
        for s in [
            Settings::default(),
            Settings {
                audio_volume: Some(87),
                screen_timeout_s: Some(15),
                locale: Some(Locale::It),
                radios: Some(RadioState {
                    wifi_on: true,
                    bt_on: false,
                    airplane_mode: false,
                    wifi_ssid: Some("home".into()),
                    wifi_ip: None,
                }),
                ..Default::default()
            },
        ] {
            let bytes = from_an_older_peer(&s);
            assert_eq!(
                crate::decode_payload(&bytes).unwrap(),
                Frame::Settings(s.clone())
            );
        }
    }

    #[test]
    fn the_new_field_survives_the_wire_and_the_json_projection() {
        for minutes in [0, 30, 720] {
            let f = Frame::SettingsSet(Settings {
                off_head_power_off_min: Some(minutes),
                ..Default::default()
            });
            let bytes = postcard::to_allocvec(&f).unwrap();
            assert_eq!(crate::decode_payload(&bytes).unwrap(), f);
        }
        let s: Settings = serde_json::from_str(
            r#"{"brightness":null,"head_wake":null,"head_wake_angle_deg":null,
                "on_head_detection":null,"audio_volume":null,"time_24h":null,"timezone_id":null,
                "power_saver":null,"deep_sleep":null,"deep_sleep_idle_s":null,"wink":null,
                "screen_timeout_s":null,"nudge":null,"locale":null,"radios":null}"#,
        )
        .unwrap();
        assert_eq!(s.off_head_power_off_min, None);
    }

    #[test]
    fn settings_frames_survive_the_wire() {
        let f = Frame::SettingsSet(Settings {
            on_head_detection: Some(true),
            timezone_id: Some(2),
            ..Default::default()
        });
        let bytes = postcard::to_allocvec(&f).unwrap();
        assert_eq!(postcard::from_bytes::<Frame>(&bytes).unwrap(), f);
    }
}
