# MyGlass feature inventory

Phone side: MyGlass **3.5.8** (`com.google.glass.companion`, versionCode 5331), referred to as `MG`; obfuscated classes are named `defpackage/<x>`.
Glass side: **XE24** system image (build 5585826), referred to as `XE24`. Field numbers refer to `Envelope` in [`protocol/proto/glass-xe24/Proto.proto`](protocol/proto/glass-xe24/Proto.proto).
Details: [`protocol/`](protocol/), [`glass-internals.md`](glass-internals.md), [`cloud-dependencies.md`](cloud-dependencies.md), [`prior-art.md`](prior-art.md).

"Rebuild" column: **A** = original companion protocol (no app on Glass), **B** = own bridge app on Glass, **—** = not rebuildable as it was (alternative given).

## Summary

| # | Feature | Cloud in MyGlass? | Rebuild | Without hardware, verifiable by |
|---|---|---|---|---|
| 1 | Pairing and connection | no | **A** | simulated Glass peer (protocol level) |
| 2 | First-run setup | yes (web setup flow) | not needed on XE24 | code reading only |
| 3 | Device info, battery, storage | no | **A** | simulated peer |
| 4 | Time zone | no (phone side optional API) | **A** | simulated peer |
| 5 | Wi-Fi setup | no | **A** | simulated peer + `SetupHelper` format tests |
| 6 | Location (phone GPS → Glass) | no | **A** (+ B for 3P apps) | simulated peer |
| 7 | Navigation | yes (Glass Maps + routing) | **B** (own nav card); A only starts a missing app | Maps-notification parser unit tests on the phone |
| 8 | Phone notifications | no | **A** via timeline items, or **B** with LiveCards | simulated peer; NotificationListener tests on AVD |
| 9 | SMS read/reply | no (Google Voice optional) | **A** | simulated peer |
| 10 | Calls | no | standard HFP (no companion work) | hardware only |
| 11 | Screencast | no | **A** | simulated peer streaming sample JPEGs |
| 12 | Remote control (touch injection) | no | **A** | simulated peer |
| 13 | Remote keyboard / text input | no | **A** | simulated peer |
| 14 | Photo/video sync | thumbnails over BT; full media via cloud **[unverified]** | **A** for notices + **B**/adb for files | simulated peer (thumbnails) |
| 15 | Contacts | yes (frontend API) | **—** → send contacts over our channel (B) or timeline share targets (A) **[to design]** | — |
| 16 | Glassware management | yes (store + sync) | **—** → sideload (adb) or B installer | — |
| 17 | Glass settings | yes (frontend API) | partial: **A** for what has messages, **B** for the rest | simulated peer |
| 18 | Internet sharing (Glass Proxy) | no | **A** (format to analyze) or BT tethering | hardware only |
| 19 | Wallpaper | no (Labs flag) | **A** if Labs `WP` enabled **[unverified]** | simulated peer |
| 20 | Media remote control | no | **—** on XE24 (field 42 not in firmware) → **B** | — |
| 21 | Unpair / debug commands | no | **A** | simulated peer |
| 22 | Factory reset (remote) | yes | **—** → on-device settings | — |

## Details

### 1. Pairing and connection
- **What:** Bluetooth bonding, then the phone connects to the Glass companion server; automatic reconnection.
- **Classes:** XE24 `bluetooth/companion/BluetoothSocketThread`, `CompanionSocketServer`, `CompanionConnection`, `ConnectionRestorer`, `PingRequester`, `bluetooth/pairing/*`; MG `defpackage/tr` (client socket), `defpackage/tq` (ping server), `service/CompanionService`.
- **Messages:** handshake (one `Envelope` each way: `version`, `androidDeviceInfo`, `glassFeatureInfoG2C` / `companionDeviceInfoC2G`, `companionFeatureInfoC2G`, `timezoneC2G`) — [`protocol/handshake.md`](protocol/handshake.md).
- **Cloud:** no. **No authentication, no Google account.**
- **Rebuild:** A. The companion opens RFCOMM secure to `F15CC914-…`, answers the handshake as version 6.10, exposes `PingCompanion` (`DC935393-…`) and reconnects on bond/ACL events.

### 2. First-run setup
- **What:** MyGlass ran a web-based setup (`setup/SetupFlowActivity`, `SetupPageWebViewClient`, `AccountVerifier`) and sent `glassSetupRequestC2G` (field 25).
- **XE24:** `GlassLiteSetup` provisions the device without account or connection, and field 25 is ignored.
- **Rebuild:** not needed. Document the manual steps (pair Bluetooth from Glass settings).

### 3. Device info, battery, storage
- **Classes:** XE24 `CompanionConnection.handleGlassInfo`, `BatteryPowerStateReceiver`; MG `service/CompanionService` (`ACTION_GLASS_INFO_REQUEST`).
- **Messages:** `glassInfoRequestC2G` (21) → `glassInfoResponseG2C` (22): battery %, storage total/free, device name, software version, needSetup, screen 640×360.
- **Rebuild:** A.

### 4. Time zone
- **Messages:** `timezoneC2G` (9), handled by `TimeZoneUtil.updateTimeZoneByCompanion`. MyGlass also called the Maps time zone API, which is not needed.
- **Rebuild:** A, sending the phone's `TimeZone.getDefault().id`.

### 5. Wi-Fi setup
- **What:** push a network from the phone; Glass may ask for a password typed on the phone.
- **Classes:** XE24 `CompanionConnection.setupGlassWifi` → broadcast `SETUP_WIFI` → `GlassHome/home/ScreenOffWifiSetupReceiver` → `util/SetupHelper.getWifiSetupInfo`; MG `base/WifiPickerActivity`, `base/InputBoxActivity`.
- **Messages:** `setupWifiC2G` (12) = `SSID\n{WPA,WEP,NONE}[\nKEY]` in plain text; `setupWifiRequestG2C` (27); `inputBoxRequestG2C`/`inputBoxResponseC2G` (23/24). MG 3.5.8 also has `wifiNetworks` (43) and `wifiStatusG2C` (44), which XE24 does not know.
- **Rebuild:** A. Note that the password travels in clear over an encrypted (bonded) RFCOMM link.

### 6. Location (phone GPS → Glass)
- **Classes:** XE24 `CompanionSocketServer.handleLocationEnvelope`, GlassHome `location/LocationService`, `CompanionLocationProxy`, `GlassLocationManager`.
- **Messages:** Glass sends `locationRequestG2C` (6) START/STOP with provider, minTime, minDistance, priority; the phone answers with `locationMessageC2G` (7) streams.
- **Rebuild:** A for Glass system features. **[unverified]** whether third-party GDK apps also see these locations through the standard `LocationManager`; if not, B adds a mock-location provider.

### 7. Navigation
- **What:** "Navigate on Glass" from the phone; turn-by-turn shown by the Glass Maps Glassware.
- **Classes:** MG `base/LaunchNavigationActivity`, `defpackage/wt`, `com/google/glass/maps/NavigationLauncher` (builds `google.navigation:q=…&mode=…&source=…`); XE24 `CompanionConnection.startNavigationFromCompanion` → `ACTION_VIEW(uri)`.
- **Messages:** `navigationRequestC2G{uri}` (10).
- **Cloud:** yes. The handler `com.google.glass.maps` is **not in the XE24 image** and relied on Google routing.
- **Rebuild:** **B** — the phone reads Google Maps' navigation notification (`NotificationListenerService`) and sends maneuver, distance, street and icon to our own 640×360 card. Option A (field 10) is useless on a stock XE24 device. An A-only fallback could push the instruction as an updating timeline item **[to evaluate in Phase 3]**.

### 8. Phone notifications
- **Classes:** MG `notification/NotificationListener`, `base/NotificationSettingsActivity`, `service/CompanionService`; XE24 `CompanionConnection.handleNotificationsList`.
- **Messages:** `notificationC2G` (45): package, key, tag, title, text, icon, dismiss.
- **Finding:** XE24 re-broadcasts field 45 as `ACTION_NOTIFICATIONS_UPDATE`, but **no consumer exists** in the decompiled XE24 APKs **[odex-only APKs not checked]**. In practice, notifications reached Glass through the Android Wear node in `MyGlassWear` (GmsCore-based) or as timeline items.
- **Rebuild:** either **A** via `timelineItem` (5) cards (native look, no app on Glass, limited actions), or **B** with LiveCards (actions, replies with dictation through `RemoteInput`). A choice for Phase 3.

### 9. SMS
- **Classes:** MG `sms/SmsReceiver`, `SmsUtils`, `SendGoogleVoiceSmsActivity`; XE24 `CompanionConnection.CompanionTimelineItemRunnable` (SMS/MMS bundle and creator handling), GlassHome `EntityProvider`.
- **Messages:** incoming SMS → `timelineItem` (5) with SMS creator/share target; replies → timeline item going back G2C **[unverified: exact reply path]**; `error` (14) for Google Voice errors.
- **Rebuild:** A.

### 10. Calls
- Standard Bluetooth Hands-Free (`0x111E/0x111F`, XE24 `bluetooth/handsfree/*`, GlassPhone). Nothing to rebuild beyond pairing.

### 11. Screencast
- **Classes:** MG `screenshare/ScreenshareActivity`; XE24 `StreamingThread` (`HiddenSurface.screenshot`, JPEG, 640×360).
- **Messages:** `screenshot` (15) start/stop C2G, frames `screenshotBytesG2C` with `isScreenOnG2C`.
- **Rebuild:** A only (a third-party app cannot capture the screen on API 19).

### 12. Remote control (touch injection)
- **Classes:** XE24 `EventInjector`; used by GlassRemote and Myo prior art.
- **Messages:** `motionC2G` (20) with an Android `MotionEvent` (action, coordinates, pointer data).
- **Rebuild:** A.

### 13. Remote keyboard / text input
- **Messages:** `inputBoxRequestG2C`/`inputBoxResponseC2G` (23/24), `openEndedInputRequestG2C`/`…ResponseC2G` (35/36); XE24 adds `inputRequestG2C`/`inputResponseC2G` (47/46) for GlassRemoteIme.
- **Classes:** MG `base/InputBoxActivity`, `base/OpenEndedInputEntryActivity`; XE24 `CompanionInputBoxResponseHandler`, GlassRemoteIme.
- **Rebuild:** A.

### 14. Photo/video sync
- **Classes:** MG `base/PhotoSyncSettingsActivity`, `ACTION_EXPIRE_PHOTOS` handling in `CompanionService`.
- **Messages:** `photoG2C` (28): thumbnail, timestamp, id, `SYNC/DELETE/NOTICE`, count; settings flags in `companionFeatureInfoC2G` (32).
- **Cloud:** full-resolution transfer is not on this channel **[unverified: likely Wi-Fi/cloud]**.
- **Rebuild:** thumbnails and notices via A; full files via B (Wi-Fi transfer) or `adb pull`.

### 15. Contacts
- **What:** choose the contacts shown on Glass for sharing, calls and messages.
- **Cloud:** yes (`GetContactsRequest`, `AddContactRequest`, … in the frontend API; Glass gets them through sync into `EntityProvider`, which needs a signature permission).
- **Rebuild:** not as it was. Alternative to design in Phase 3: B keeps its own contact list, or A sends entities inside timeline items.

### 16. Glassware management
- **Cloud:** yes (store, enable/disable, server-driven install). Backend is gone.
- **Rebuild:** not as it was. Alternative: sideload with adb (developer mode) or an installer in the B app.

### 17. Glass settings
- **Cloud:** MyGlass edited many settings through the frontend API (`GetSettingsRequest`/`SetSettingsRequest`).
- **Rebuild:** settings that have protocol messages (Wi-Fi, time zone, photo sync, wallpaper) via A; others via B or on-device. `SyncableSettingsProvider` on Glass is exported **[unverified: content]**.

### 18. Internet sharing (Glass Proxy)
- **Classes:** MG `proxy/ProxyService`, `defpackage/ry` (server `Glass Proxy`, UUID `fafbdd20-…`).
- **Rebuild:** A after analyzing the proxy stream format, or standard BT tethering (NAP `0x1116`) **[to analyze]**.

### 19. Wallpaper
- **Messages:** 34, 38–41; enabled only with the Labs flag `WP` on Glass. MG `base/WallpaperActivity`, `service/WallpaperProcessingService`.
- **Rebuild:** low priority.

### 20. Media remote control
- MG 3.5.8 field 42 `mediaRemoteControlC2G` is unknown to XE24, so it does not work on this firmware. AnotherGlass implements it with its own protocol (B).

### 21. Unpair / debug
- `command` (16): `UNPAIR`, `START_DEBUG`, `STOP_DEBUG`. Rebuild: A.

### 22. Factory reset (remote)
- Frontend API only (`FactoryResetRequest`). Rebuild: no; reset from Glass settings.

## Hardware-free verifiability

Per [`emulator-feasibility.md`](emulator-feasibility.md), Glass firmware cannot be emulated here. Without hardware we can verify:
- **Protocol conformance** of our phone code against a simulated Glass peer that replays the handshake and message sequences documented in [`protocol/`](protocol/) (features 1, 3–6, 8, 9, 11–13, 17, 19, 21).
- **Phone-side logic** on an Android emulator: NotificationListener parsing (including Google Maps navigation notifications, feature 7), location capture, Wi-Fi string building.
- **Bridge-app logic** (if B is chosen) on an arm64 AVD with GDK stubs.

Real-firmware behavior (whether XE24 accepts our handshake, renders our timeline items, streams screencast) can only be confirmed on a device, using HCI snoop logs and `logcat`.
