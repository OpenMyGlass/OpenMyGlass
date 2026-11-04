//! A reusable **simulated Glass** peer.
//!
//! It plays the Glass side of the link: completes the handshake (accepting a one-time
//! pairing), then serves the backbone messages. It exists so the companion link can be
//! driven end-to-end with no hardware — in the integration tests and behind the CLI's
//! `sim-glass` command.
//!
//! This is also a precise, executable description of the **backbone glassd must
//! implement** to match the companion: see each `match` arm.

use omg_protocol::{
    BatteryStatus, Frame, GlassStatus, Locale, LocaleChanged, NotificationAction,
    NotificationChoice, RadioState, Settings,
};
use omg_transport::{Link, Stream};
use tracing::{debug, info};

use crate::{
    identity::Identity,
    session::{PairingMode, Session, SessionConfig},
    store::PairingRecord,
    CoreError,
};

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
        }
    }
}

/// Run a simulated Glass over `link`, bonded to `bonded` (or `None` to accept a new
/// pairing). Returns when the companion disconnects.
pub async fn run<S: Stream>(
    link: Link<S>,
    identity: Identity,
    bonded: Option<PairingRecord>,
    mut state: SimGlassState,
) -> Result<PairingRecord, CoreError> {
    let cfg = SessionConfig::glass(state.device_name.clone());
    let mut session = Session::new(link, identity, bonded, cfg);

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

    // Push an initial battery status, like glassd's BatteryPowerStateReceiver.
    session
        .send(Frame::BatteryStatus(BatteryStatus {
            percent: state.battery_percent,
            charging: state.charging,
        }))
        .await?;

    while let Some(frame) = session.recv_app().await? {
        match frame {
            Frame::StatusRequest(_) => {
                session
                    .send(Frame::Status(GlassStatus {
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
                    }))
                    .await?;
            }
            Frame::SettingsGet | Frame::SettingsSet(_) => {
                if let Frame::SettingsSet(change) = &frame {
                    state.settings.merge(change);
                }
                session
                    .send(Frame::Settings(state.settings.clone()))
                    .await?;
            }
            Frame::SetLocale(set) => {
                state.locale = set.locale;
                debug!(locale = set.locale.code(), "sim-glass: locale set");
                session
                    .send(Frame::LocaleChanged(LocaleChanged {
                        locale: state.locale,
                    }))
                    .await?;
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
                    session
                        .send(Frame::NotificationAction(NotificationAction {
                            key: n.notification.key.clone(),
                            choice: NotificationChoice::Dismiss,
                        }))
                        .await?;
                }
            }
            Frame::NotificationRemoved(r) => {
                debug!(key = %r.key, "sim-glass: notification removed");
            }
            Frame::SetRadio(_) | Frame::SetAirplaneMode(_) => {
                // SCAFFOLD: a real glassd drives rfkill/wpa_supplicant/bluez here.
                session
                    .send(Frame::RadioState(RadioState {
                        wifi_on: false,
                        bt_on: true,
                        airplane_mode: false,
                        wifi_ssid: None,
                        wifi_ip: None,
                    }))
                    .await?;
            }
            other => {
                debug!(frame = other.wire_name(), "sim-glass: unhandled (scaffold)");
            }
        }
    }

    // Our side is Glass, so from its perspective the bonded peer is the companion.
    debug!("sim-glass: companion disconnected");
    Ok(session.bonded().cloned().unwrap_or(rec))
}
