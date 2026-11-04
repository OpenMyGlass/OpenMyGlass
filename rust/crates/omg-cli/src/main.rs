//! `omg` — the OpenMyGlass companion CLI/daemon.
//!
//! This is a thin frontend over the shared Rust core (`omg-core`). The same core will
//! sit behind the Android/desktop UI later. Commands:
//!
//! * `omg identity`   — show (create on first use) the companion's identity key.
//! * `omg qr`         — generate an offline Wi-Fi bootstrap QR for the Glass camera.
//! * `omg sim-glass`  — run a simulated Glass peer (for pairing/testing without hardware).
//! * `omg link`       — connect, pair if needed, and run the backbone once (demo).
//! * `omg daemon`     — connect and keep the link up (heartbeat + event stream).
//! * `omg note-add`   — seal a note into a simulated Glass's queue (works offline).
//! * `omg notes-sync` — connect and pull the Glass's waiting notes, resuming after drops.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use omg_core::{
    notes::{
        run_companion, AttachmentSource, CompanionNotes, CompanionRunOptions, CompanionUpdate,
        DeletePolicy, GlassNotes, LinkProfile, NoEngine, NoteDraft, ReceiverEvent, SenderConfig,
        SpeechError, SpeechToText, SynthesizeRequest, SynthesizedAudio, TextToSpeech,
        TranscribeRequest,
    },
    session::{PairingMode, Session, SessionConfig},
    simglass::{self, SimGlassState, SimNoteFaults, SimNotes},
    Store,
};
use omg_protocol::{
    bootstrap::{WifiAuth, WifiBootstrap},
    capability, AttachmentKind, Frame, Locale, Notification, NotificationPosted, SetLocale,
    Settings, StatusRequest,
};
use omg_qr::BootstrapQr;
use omg_transport::{TcpListenerTransport, TcpTransport, Transport};
use tracing::info;

#[derive(Parser)]
#[command(name = "omg", version, about = "OpenMyGlass companion (phone/PC)")]
struct Cli {
    /// Path to the persistent state file (identity + bonded peer).
    #[arg(long, global = true)]
    store: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the companion identity (created on first use).
    Identity,
    /// Generate an offline Wi-Fi bootstrap QR for the Glass camera.
    Qr(QrArgs),
    /// Run a simulated Glass peer (listens for a companion).
    SimGlass(ListenArgs),
    /// Connect to a Glass, pair if needed, and run the backbone once.
    Link(ConnectArgs),
    /// Connect and keep the link up (heartbeat + event stream).
    Daemon(ConnectArgs),
    /// Seal a note into a simulated Glass's queue. Needs no companion: it waits there.
    NoteAdd(NoteAddArgs),
    /// Connect to a Glass and pull its waiting notes, resuming after a dropped link.
    NotesSync(NotesSyncArgs),
}

#[derive(Args)]
struct QrArgs {
    /// Wi-Fi SSID.
    #[arg(long)]
    ssid: String,
    /// Pre-shared key (omit for an open network).
    #[arg(long)]
    psk: Option<String>,
    /// Authentication type.
    #[arg(long, value_enum, default_value_t = AuthKind::Wpa)]
    auth: AuthKind,
    /// Direct companion address hint `ip:port` (otherwise the Glass uses mDNS).
    #[arg(long)]
    addr: Option<String>,
    /// mDNS service to discover the companion by.
    #[arg(long, default_value = "_openmyglass._tcp.local")]
    mdns: String,
    /// One-time pairing bootstrap token to embed.
    #[arg(long)]
    token: Option<String>,
    /// Also write the QR as SVG to this file.
    #[arg(long)]
    svg: Option<PathBuf>,
}

#[derive(Copy, Clone, ValueEnum)]
enum AuthKind {
    Open,
    Wep,
    Wpa,
    Wpa3,
}

#[derive(Args)]
struct ListenArgs {
    /// Address to listen on, e.g. `0.0.0.0:7700`.
    #[arg(long, default_value = "127.0.0.1:7700")]
    listen: String,
    /// Friendly device name to advertise.
    #[arg(long, default_value = "OpenMyGlass-Sim")]
    name: String,
    /// Keep a note queue in this directory and sync it (see `note-add`).
    #[arg(long)]
    notes_dir: Option<PathBuf>,
    /// Delete a note's files here once the companion has verified all of it.
    #[arg(long, requires = "notes_dir")]
    delete_after_sync: bool,
    /// Ask the companion to read each transcript back, to exercise TTS.
    #[arg(long, requires = "notes_dir")]
    read_back: bool,
    /// Fault, first connection only: drop the link after this many attachment chunks.
    #[arg(long, requires = "notes_dir")]
    drop_after_chunks: Option<u32>,
    /// Fault, first connection only: damage this attachment chunk (1-based).
    #[arg(long, requires = "notes_dir")]
    corrupt_chunk: Option<u32>,
}

#[derive(Args)]
struct NoteAddArgs {
    /// The simulated Glass's queue directory.
    #[arg(long)]
    notes_dir: PathBuf,
    /// Note text.
    #[arg(long)]
    text: Option<String>,
    /// The voice recording.
    #[arg(long)]
    voice: Option<PathBuf>,
    /// A photo. Repeat for more.
    #[arg(long)]
    photo: Vec<PathBuf>,
    /// A video.
    #[arg(long)]
    video: Option<PathBuf>,
    /// Language of the note.
    #[arg(long, value_enum, default_value_t = LocaleArg::En)]
    locale: LocaleArg,
}

#[derive(Copy, Clone, ValueEnum)]
enum LinkArg {
    /// As for Bluetooth LE.
    Constrained,
    /// As for Bluetooth Classic.
    Moderate,
    /// As for Wi-Fi.
    Fast,
}

#[derive(Args)]
struct NotesSyncArgs {
    /// Glass address to connect to.
    #[arg(long)]
    connect: String,
    /// Friendly name of this companion.
    #[arg(long, default_value = "openmyglass-companion")]
    name: String,
    /// Where notes received on this companion are kept.
    #[arg(long)]
    notes_dir: PathBuf,
    /// Chunk size and window to use. This is TCP either way; it only sets the profile.
    #[arg(long, value_enum, default_value_t = LinkArg::Fast)]
    link: LinkArg,
    /// Use placeholder speech engines, to exercise the flow. Without this flag there is
    /// no engine, as none ships with the core, and transcripts come back as unavailable.
    #[arg(long)]
    fake_speech: bool,
    /// Connect again up to this many times if the link drops before the sync is done.
    #[arg(long, default_value_t = 0)]
    reconnect: u32,
}

#[derive(Args)]
struct ConnectArgs {
    /// Glass address to connect to, e.g. `10.10.10.42:7700`.
    #[arg(long)]
    connect: String,
    /// Friendly name of this companion.
    #[arg(long, default_value = "openmyglass-companion")]
    name: String,
    /// Force a (re)pairing even if a peer is already bonded.
    #[arg(long)]
    pair: bool,
    /// After the handshake, push this UI locale to the Glass.
    #[arg(long, value_enum)]
    locale: Option<LocaleArg>,
    /// After the handshake, read the Glass settings.
    #[arg(long)]
    get_settings: bool,
    /// After the handshake, set the earpiece volume (0-100).
    #[arg(long)]
    volume: Option<u8>,
    /// After the handshake, set 24-hour (true) or 12-hour (false) time.
    #[arg(long)]
    time_24h: Option<bool>,
    /// After the handshake, set the time zone (index in the Glass's table).
    #[arg(long)]
    timezone: Option<u32>,
    /// After the handshake, turn on-head detection on or off.
    #[arg(long)]
    on_head: Option<bool>,
    /// After the handshake, set the minutes off the head before powering off (0 = never).
    #[arg(long)]
    off_head_power_off_min: Option<u32>,
}

#[derive(Copy, Clone, ValueEnum)]
enum LocaleArg {
    It,
    En,
    Fr,
    Es,
}

impl From<LocaleArg> for Locale {
    fn from(l: LocaleArg) -> Self {
        match l {
            LocaleArg::It => Locale::It,
            LocaleArg::En => Locale::En,
            LocaleArg::Fr => Locale::Fr,
            LocaleArg::Es => Locale::Es,
        }
    }
}

fn default_store_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config/openmyglass/companion.json")
    } else {
        PathBuf::from("omg-state.json")
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "omg=info,omg_core=info,omg_transport=warn".into()),
        )
        .init();

    let cli = Cli::parse();
    let store_path = cli.store.unwrap_or_else(default_store_path);

    match cli.cmd {
        Command::Identity => cmd_identity(&store_path),
        Command::Qr(args) => cmd_qr(args),
        Command::SimGlass(args) => cmd_sim_glass(&store_path, args).await,
        Command::Link(args) => cmd_link(&store_path, args).await,
        Command::Daemon(args) => cmd_daemon(&store_path, args).await,
        Command::NoteAdd(args) => cmd_note_add(args),
        Command::NotesSync(args) => cmd_notes_sync(&store_path, args).await,
    }
}

fn cmd_identity(store_path: &Path) -> Result<()> {
    let store = Store::load_or_create(store_path).context("load store")?;
    println!("store:    {}", store_path.display());
    println!("identity: {}", store.identity().public_b64());
    match store.paired() {
        Some(p) => println!("paired:   {} ({})", p.peer_name, p.peer_pub),
        None => println!("paired:   <none> (run `omg link --pair` to bond)"),
    }
    Ok(())
}

fn cmd_qr(args: QrArgs) -> Result<()> {
    let auth = match args.auth {
        AuthKind::Open => WifiAuth::Open,
        AuthKind::Wep => WifiAuth::Wep,
        AuthKind::Wpa => WifiAuth::WpaPsk,
        AuthKind::Wpa3 => WifiAuth::Wpa3Sae,
    };
    let bootstrap = WifiBootstrap {
        version: omg_protocol::bootstrap::BOOTSTRAP_VERSION,
        ssid: args.ssid,
        auth,
        psk: args.psk,
        hidden: false,
        companion_addr: args.addr,
        mdns_service: Some(args.mdns),
        pair_token: args.token,
    };
    let qr = BootstrapQr::from_bootstrap(&bootstrap).context("render QR")?;
    println!("{}", qr.to_terminal());
    println!("payload: {}", qr.payload);
    println!("(Glass decodes the OMG1: payload; see omg_protocol::bootstrap)");
    if let Some(path) = args.svg {
        std::fs::write(&path, qr.to_svg()).with_context(|| format!("write {}", path.display()))?;
        println!("svg:     {}", path.display());
    }
    Ok(())
}

async fn cmd_sim_glass(store_path: &Path, args: ListenArgs) -> Result<()> {
    let listener = TcpListenerTransport::bind(&args.listen)
        .await
        .with_context(|| format!("bind {}", args.listen))?;
    info!(addr = %args.listen, "sim-glass listening");

    let mut faults = SimNoteFaults {
        drop_link_after_chunks: args.drop_after_chunks,
        corrupt_chunk: args.corrupt_chunk,
    };
    loop {
        let link = listener.accept().await.context("accept")?;
        // Reload the store each connection so a fresh pairing persists for the next one.
        let mut store = Store::load_or_create(store_path)?;
        let notes = args.notes_dir.as_ref().map(|dir| SimNotes {
            delete_policy: if args.delete_after_sync {
                DeletePolicy::DeleteAfterVerified
            } else {
                DeletePolicy::Keep
            },
            faults: std::mem::take(&mut faults),
            tts_after_transcript: args.read_back,
            ..SimNotes::new(dir)
        });
        let state = SimGlassState {
            device_name: args.name.clone(),
            notes,
            ..Default::default()
        };
        let id = store.identity().clone();
        let bonded = store.paired().cloned();
        match simglass::run(link, id, bonded, state).await {
            Ok(rec) => {
                if store.paired().is_none() {
                    store.set_paired(rec.peer_pub.clone(), rec.peer_name.clone())?;
                    info!(companion = %rec.peer_name, "sim-glass: bonded and persisted");
                }
            }
            Err(e) => info!(error = %e, "sim-glass: session ended"),
        }
    }
}

/// Dial, handshake (pairing if needed), and persist a new bond. Returns the session.
async fn connect_and_handshake(
    store: &mut Store,
    args: &ConnectArgs,
) -> Result<Session<omg_transport::BoxStream>> {
    dial_and_handshake(store, &args.connect, &args.name, args.pair, &[]).await
}

/// Dial, advertise `capabilities` on top of the backbone, and handshake.
async fn dial_and_handshake(
    store: &mut Store,
    connect: &str,
    name: &str,
    pair: bool,
    capabilities: &[&str],
) -> Result<Session<omg_transport::BoxStream>> {
    let link = TcpTransport::new(connect)
        .dial()
        .await
        .with_context(|| format!("connect {connect}"))?;

    let was_paired = store.paired().is_some();
    let mode = if pair || !was_paired {
        PairingMode::AllowNew
    } else {
        PairingMode::ReuseOnly
    };

    let mut session = Session::new(
        link,
        store.identity().clone(),
        store.paired().cloned(),
        SessionConfig::companion(name),
    );
    for capability in capabilities {
        session.advertise(capability);
    }
    let rec = session.handshake(mode).await.context("handshake")?;
    if !was_paired || pair {
        store.set_paired(rec.peer_pub.clone(), rec.peer_name.clone())?;
        info!(glass = %rec.peer_name, "paired and persisted");
    } else {
        info!(glass = %rec.peer_name, "reconnected to bonded Glass");
    }
    Ok(session)
}

async fn cmd_link(store_path: &Path, args: ConnectArgs) -> Result<()> {
    let mut store = Store::load_or_create(store_path)?;
    let mut session = connect_and_handshake(&mut store, &args).await?;

    // Ask for status, push the locale (optional), and forward a demo notification.
    session
        .send(Frame::StatusRequest(StatusRequest::default()))
        .await?;
    if let Some(l) = args.locale {
        session
            .send(Frame::SetLocale(SetLocale { locale: l.into() }))
            .await?;
    }
    let change = Settings {
        audio_volume: args.volume,
        time_24h: args.time_24h,
        timezone_id: args.timezone,
        on_head_detection: args.on_head,
        off_head_power_off_min: args.off_head_power_off_min,
        ..Default::default()
    };
    if change != Settings::default() {
        session.send(Frame::SettingsSet(change)).await?;
    }
    if args.get_settings {
        session.send(Frame::SettingsGet).await?;
    }
    session
        .send(Frame::NotificationPosted(NotificationPosted {
            notification: Notification {
                key: "demo-1".into(),
                app_name: "OpenMyGlass".into(),
                title: "Linked".into(),
                text: "Backbone is up".into(),
                posted_at_ms: omg_core::now_ms(),
                icon_sha256: None,
                actions: vec![],
                wake: false,
            },
        }))
        .await?;

    // Drain the replies (battery push, status, locale ack, notif action).
    for _ in 0..5 {
        match tokio::time::timeout(Duration::from_millis(400), session.recv_app()).await {
            Ok(Ok(Some(frame))) => print_event(&frame),
            Ok(Ok(None)) => break,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => break, // no more frames within the window
        }
    }

    // Heartbeat last, so the pong is not racing the backbone replies.
    let rtt = session.heartbeat().await?;
    println!("heartbeat: {rtt} ms");

    session.close("done").await?;
    Ok(())
}

async fn cmd_daemon(store_path: &Path, args: ConnectArgs) -> Result<()> {
    let mut store = Store::load_or_create(store_path)?;
    let mut session = connect_and_handshake(&mut store, &args).await?;
    if let Some(l) = args.locale {
        session
            .send(Frame::SetLocale(SetLocale { locale: l.into() }))
            .await?;
    }
    session
        .send(Frame::StatusRequest(StatusRequest::default()))
        .await?;
    info!("daemon running; Ctrl-C to stop");

    let mut hb = tokio::time::interval(Duration::from_secs(15));
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("shutting down");
                session.close("companion shutting down").await.ok();
                break;
            }
            _ = hb.tick() => {
                match session.heartbeat().await {
                    Ok(rtt) => info!(rtt_ms = rtt, "heartbeat"),
                    Err(e) => { info!(error = %e, "heartbeat failed, link lost"); break; }
                }
            }
            frame = session.recv_app() => {
                match frame {
                    Ok(Some(f)) => print_event(&f),
                    Ok(None) => { info!("link closed by Glass"); break; }
                    Err(e) => { info!(error = %e, "link error"); break; }
                }
            }
        }
    }
    Ok(())
}

fn cmd_note_add(args: NoteAddArgs) -> Result<()> {
    let source = |kind, mime: &str, path: &PathBuf| AttachmentSource {
        kind,
        mime: mime.to_string(),
        path: path.clone(),
        duration_ms: None,
    };
    let mut attachments = Vec::new();
    if let Some(path) = &args.voice {
        attachments.push(source(AttachmentKind::VoiceRecording, "audio/wav", path));
    }
    for path in &args.photo {
        attachments.push(source(AttachmentKind::Photo, "image/jpeg", path));
    }
    if let Some(path) = &args.video {
        attachments.push(source(AttachmentKind::Video, "video/mp4", path));
    }
    let draft = NoteDraft {
        created_at_ms: omg_core::now_ms(),
        locale: args.locale.into(),
        text: args.text,
        attachments,
    };
    let mut queue = GlassNotes::open(&args.notes_dir, SenderConfig::default())?;
    let (id, _) = queue.add_note(&draft, omg_core::now_ms())?;
    let stats = queue.machine().stats();
    println!("note {} sealed", id.to_hex());
    println!(
        "waiting on Glass: {} note(s), {} byte(s)",
        stats.pending_notes, stats.pending_bytes
    );
    Ok(())
}

/// Stand-in engines for trying the flow. No real speech happens here.
struct FakeSpeech;

impl SpeechToText for FakeSpeech {
    fn transcribe(&self, request: &TranscribeRequest) -> Result<String, SpeechError> {
        let bytes = std::fs::metadata(&request.audio_path)
            .map_err(|e| SpeechError::Failed(e.to_string()))?
            .len();
        Ok(format!(
            "[fake transcript of {bytes} bytes of {}]",
            request.mime
        ))
    }
}

impl TextToSpeech for FakeSpeech {
    fn synthesize(&self, request: &SynthesizeRequest) -> Result<SynthesizedAudio, SpeechError> {
        // Silence, a little of it per character.
        Ok(SynthesizedAudio {
            sample_rate: 16_000,
            pcm: vec![0; 2_000 + request.text.len() * 64],
        })
    }
}

async fn cmd_notes_sync(store_path: &Path, args: NotesSyncArgs) -> Result<()> {
    let mut store = Store::load_or_create(store_path)?;
    let mut notes = CompanionNotes::open(&args.notes_dir, Default::default())?;
    for id in notes.set_aside() {
        println!("note {id}: its record is unreadable; set aside on disk");
    }
    let (stt, tts): (Arc<dyn SpeechToText>, Arc<dyn TextToSpeech>) = if args.fake_speech {
        (Arc::new(FakeSpeech), Arc::new(FakeSpeech))
    } else {
        (Arc::new(NoEngine), Arc::new(NoEngine))
    };
    let profile = match args.link {
        LinkArg::Constrained => LinkProfile::constrained(),
        LinkArg::Moderate => LinkProfile::moderate(),
        LinkArg::Fast => LinkProfile::fast(),
    };
    let options = CompanionRunOptions {
        profile,
        close_when_idle_for: Some(Duration::from_millis(500)),
    };

    let mut attempts_left = args.reconnect;
    loop {
        let outcome = sync_once(&mut store, &mut notes, &args, &stt, &tts, options).await;
        match outcome {
            Ok(()) if notes.machine().is_idle() => break,
            Ok(()) => println!("link closed before the sync finished"),
            Err(e) => println!("link failed: {e:#}"),
        }
        if attempts_left == 0 {
            anyhow::bail!("gave up with the sync unfinished; the notes stay on the Glass");
        }
        attempts_left -= 1;
        tokio::time::sleep(Duration::from_millis(200)).await;
        println!("reconnecting");
    }

    let complete = notes.machine().notes().filter(|n| n.complete).count();
    println!(
        "complete here: {complete} note(s) in {}",
        args.notes_dir.display()
    );
    if let Some(status) = notes.machine().peer_status() {
        println!(
            "still on Glass: {} note(s), {} byte(s)",
            status.pending_notes, status.pending_bytes
        );
    }
    Ok(())
}

async fn sync_once(
    store: &mut Store,
    notes: &mut CompanionNotes,
    args: &NotesSyncArgs,
    stt: &Arc<dyn SpeechToText>,
    tts: &Arc<dyn TextToSpeech>,
    options: CompanionRunOptions,
) -> Result<()> {
    let session = dial_and_handshake(
        store,
        &args.connect,
        &args.name,
        false,
        &[capability::NOTES_SYNC],
    )
    .await?;
    if !session.peer_supports(capability::NOTES_SYNC) {
        anyhow::bail!("this Glass does not speak note sync");
    }
    run_companion(
        session.into_link(),
        notes,
        stt.clone(),
        tts.clone(),
        options,
        print_note_update,
    )
    .await?;
    Ok(())
}

fn print_note_update(update: CompanionUpdate<'_>) {
    let CompanionUpdate::Note(event) = update else {
        return;
    };
    match event {
        ReceiverEvent::NoteStarted { note_id } => println!("note {}: started", note_id.to_hex()),
        ReceiverEvent::AttachmentVerified {
            note_id,
            attachment_id,
        } => println!(
            "note {}: attachment {attachment_id} verified",
            note_id.to_hex()
        ),
        ReceiverEvent::NoteComplete { note_id } => println!("note {}: complete", note_id.to_hex()),
        ReceiverEvent::ChunkRejected {
            note_id,
            offset,
            reason,
            ..
        } => println!(
            "note {}: chunk at {offset} rejected ({reason:?})",
            note_id.to_hex()
        ),
        ReceiverEvent::AttachmentHashFailed {
            note_id,
            attachment_id,
        } => println!(
            "note {}: attachment {attachment_id} failed its checksum",
            note_id.to_hex()
        ),
        ReceiverEvent::NoteDeferred { note_id, reason } => {
            println!("note {}: deferred ({reason:?})", note_id.to_hex())
        }
        ReceiverEvent::NoteRejected { note_id, reason } => {
            println!("note {}: rejected ({reason:?})", note_id.to_hex())
        }
        ReceiverEvent::TranscriptReady {
            note_id, result, ..
        } => match &result.error {
            None => println!("note {}: transcript: {}", note_id.to_hex(), result.text),
            Some(error) => println!("note {}: no transcript ({error:?})", note_id.to_hex()),
        },
    }
}

fn print_event(frame: &Frame) {
    match frame {
        Frame::Status(s) => println!(
            "status: battery={:?}% charging={:?} ver={:?} locale={:?}",
            s.battery_percent, s.charging, s.software_version, s.locale
        ),
        Frame::BatteryStatus(b) => println!("battery: {}% charging={}", b.percent, b.charging),
        Frame::LocaleChanged(l) => println!("locale now: {}", l.locale.code()),
        Frame::Settings(s) => println!("settings: {s:?}"),
        Frame::NotificationAction(a) => println!("notif action on {}: {:?}", a.key, a.choice),
        Frame::RadioState(r) => println!("radios: {r:?}"),
        other => println!("event: {}", other.wire_name()),
    }
}
