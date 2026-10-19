# Glass-side internal APIs (XE24)

What the companion drives on Glass, and what a **third-party app on the glasses** (a GDK bridge app) could reach instead.
Sources: decoded manifests in `_re/apktool/<app>/AndroidManifest.xml` and decompiled code in `_re/jadx/<app>` (XE24, build 5585826).

Reachability legend: **3P** = any third-party app · **sig** = needs the platform/app signature (`signature` / `signatureOrSystem`) · **sys** = system UID or root.

## The companion service itself

| Component | App | Permission | Reach | Notes |
|---|---|---|---|---|
| `GlassBluetoothService` (action `com.google.glass.bluetooth.COMPANION_SERVICE`) | GlassBluetooth (`android.uid.system`) | `com.google.glass.bluetooth.permission.COMPANION` (**signatureOrSystem**) | sig | Hosts the RFCOMM companion server. Glass apps talk to it through `IRemoteCompanionService` / `RemoteCompanionProxy` (Messenger-based; envelopes passed as bytes). A 3P app on Glass **cannot** bind it. |

Broadcasts the companion server emits on Glass after receiving an envelope (`CompanionConnection`):

| Broadcast | Payload | Permission | Receiver on XE24 |
|---|---|---|---|
| `com.google.glass.companion.SETUP_WIFI` (extra `wifi_setup_string`) | `SSID\n{WPA,WEP,NONE}[\nKEY]` | `…bluetooth.permission.COMPANION` | `GlassHome/home/ScreenOffWifiSetupReceiver` |
| `com.google.glass.companion.ACTION_NOTIFICATIONS_UPDATE` (extra `notification_update_bytes`) | `Proto.Notification` | `…bluetooth.permission.COMPANION` | **none found** [unverified for odex-only APKs] |
| `com.google.glass.remoteime.IME_SERVICE` (extra `com.google.glass.remoteime.INPUT`) | `Proto.InputResponse` | none (implicit broadcast) | GlassRemoteIme |
| `android.intent.action.VIEW` with the navigation URI | `google.navigation:…` | — | **no handler in the XE24 image** (see Navigation) |

## Timeline

| Component | App | Permission | Reach | Notes |
|---|---|---|---|---|
| `TimelineProvider` (authority `com.google.glass.sync.timeline`) | GlassSync | `com.google.glass.sync.permission.TIMELINE_PROVIDER` (**signatureOrSystem**) | sig | Native timeline storage. The companion inserts cards here through `CompanionSocketServer.insertTimelineItem` (Envelope field 5). |
| `MainTimelineActivity` (`com.google.android.glass.action.REVEAL_CARD`, `…NAVIGATE_TO_CARD`) | GlassHome | none | 3P | Can be launched to show a card. |
| `SingleNotificationActivity` / `BundleNotificationActivity` (`com.google.glass.ACTION_SHOW_SINGLE_TIMELINE_NOTIFICATION`, `…BUNDLE…`) | GlassHome | none | 3P | Shows timeline notifications; inputs are timeline item ids. |
| `FullScreenMessagingDialogActivity` (`com.google.glass.action.MESSAGE`) | GlassHome | none | 3P | Full-screen message. |
| `EntityProvider` (`com.google.glass.entity`) | GlassHome | `ENTITY_PROVIDER` (**signature**) | sig | Contacts/entities used by sharing and SMS. |

**Consequence:** a third-party app on Glass cannot write native timeline cards. It must use the public GDK surfaces (`LiveCard`, immersions, `CardBuilder`). The companion channel (option A) **can** write native cards, SMS included.

## Notifications

- The XE24 companion path for phone notifications (Envelope field 45) ends in a broadcast with no consumer (see above).
- Working native paths: **timeline items** (field 5, e.g. SMS) and the **Android Wear node** in `MyGlassWear` (`NotificationCollectorService`, `NotificationBridgerService`, `DispatchingWearableListenerService` bound by GmsCore with `com.google.android.gms.wearable.BIND_LISTENER`). The Wear path requires Google Play services on both devices and a Wear-capable phone app **[unverified whether current Wear OS phone apps still pair with it]**.
- A 3P bridge app can render notifications as LiveCards, needing no system permission.

## Location

- GlassHome `com.google.glass.location.LocationService` (permission `ACCESS_FINE_LOCATION`) merges a **local** provider (Android `LocationManager`) and a **remote** one fed by the companion (`CompanionLocationProxy`, Envelope fields 6/7). Clients use the Glass-internal `GlassLocationManager` over a Messenger.
- **[unverified]** Whether companion locations also reach the standard Android `LocationManager` for third-party GDK apps. If not, a bridge app has to forward phone GPS as a mock location (`addTestProvider` needs "allow mock locations" in developer options) or deliver it over its own channel.

## Navigation

- MyGlass sends `NavigationRequest{uri}` with a `google.navigation:q=…&mode=…&source=…` URI (MG 3.5.8: `com/google/android/apps/myglass/base/LaunchNavigationActivity` forwards a URI it receives; MG `defpackage/wt` takes the URI built by `com/google/glass/maps/NavigationLauncher.getNavigationIntent`); Glass fires `ACTION_VIEW(uri)`.
- **No APK in the XE24 system image declares a `VIEW` intent filter.** Native turn-by-turn lived in the separate Glassware package `com.google.glass.maps` (referenced by GlassHome, GlassSync, GlassSettings, MyGlassWear; e.g. `com.google.glass.maps.service.MapRenderingService`), which is not in the image.
- So on a stock XE24 device the navigation request does nothing unless `com.google.glass.maps` is installed, and that app depended on Google routing servers (see [`cloud-dependencies.md`](cloud-dependencies.md)).

## Camera, voice, settings (launchable by 3P)

| Action | App | Reach |
|---|---|---|
| `com.google.glass.action.TAKE_PICTURE`, `…RECORD_VIDEO`, `…TAKE_PICTURE_FROM_SCREEN_OFF`; `ApiTakePictureActivity`, `ApiRecordVideoActivity` | GlassCamera | 3P |
| `com.google.android.glass.action.VOICE_TRIGGER` (GDK voice trigger) | GlassCamera, … | 3P (GDK public API) |
| `com.google.glass.action.ACTION_GO_TO_SETTINGS`, `…SETUP_BLUETOOTH`, `…WIFI_PASSWORD` | GlassSettings | 3P |
| `com.google.glass.action.SETUP_WIFI` (`WifiSelectorActivity`) | GlassSettings | sig (`com.google.glass.home.permission.SETUP_WIFI`, signature) |
| `SyncableSettingsProvider` (`com.google.glass.settings.syncable`) | GlassSettings (`android.uid.system`) | exported, no permission — **[unverified]** which settings it exposes |

## Screencast and input injection

- Screencast frames come from `HiddenSurface.screenshot` (hidden API) in `StreamingThread`, and touch injection from `EventInjector` (hidden input API), both inside the system-UID GlassBluetooth. A 3P app **cannot** capture the screen or inject input on API 19 without root (no MediaProjection before API 21).
- **Consequence:** screencast and remote control are only possible through the original companion channel (option A) or with root.

## Summary for Phase 3

| Native capability | Via companion channel (option A) | Via 3P bridge app (option B) |
|---|---|---|
| Native timeline cards / SMS | ✅ | ❌ (LiveCards instead) |
| Wi-Fi setup | ✅ | ⚠️ needs Wi-Fi APIs from a normal app (`WifiManager` is public on API 19) |
| Screencast / touch injection | ✅ | ❌ without root |
| Device info / battery | ✅ | ✅ (public APIs) |
| Location for Glass apps | ✅ (Glass-internal path) | ⚠️ mock location or own channel |
| Turn-by-turn native nav | ❌ (needs `com.google.glass.maps` + cloud) | ✅ own navigation card |
| Phone notifications | ⚠️ field 45 unconsumed; timeline items work | ✅ LiveCards with actions |
