# OpenMyGlass architecture (Phase 3)

Status: **approved by the owner** (2026-10-19), updated with the owner's answers on navigation source, Android Auto, hardware, offline transcription, the optional minimap and iOS readiness. Inputs: [`project-brief.md`](project-brief.md), [`feature-inventory.md`](feature-inventory.md), [`protocol/`](protocol/), [`glass-internals.md`](glass-internals.md), [`cloud-dependencies.md`](cloud-dependencies.md), [`prior-art.md`](prior-art.md), [`emulator-feasibility.md`](emulator-feasibility.md).

## 1. Intent

A local, cloud-free replacement for MyGlass for one Glass Explorer Edition on **XE24**, paired with an Android phone. The owner uses Glass for, in order of importance:

1. **Navigation, by car and on foot**: the next maneuver (arrow, distance, street) on the glasses, with the phone mounted in the car or in a pocket with the screen off, and **also while the phone is connected to Android Auto** (the owner sometimes drives with it and navigation must keep working). The owner prefers a **navigation app that exposes an API** over scraping another app's notification.
2. **Notifications**: read them on the glasses and, where the app allows it, **reply by dictation**. Glass only records audio; **the phone transcribes, offline only** (no cloud speech service), to save Glass battery.
3. **Photos and videos** taken hands-free with the native camera (rare). They **sync automatically to the phone gallery when Glass is charging and on Wi-Fi**, and otherwise are moved on demand.

**Glass battery life is an explicit constraint** set by the owner.

**No Glass hardware is available yet.** A phase counts as done when it passes its tests against the simulated Glass peer and the emulators; what can only be confirmed on a device goes into the hardware checklist (§11) and is verified when Glass is available.

### Goals
- **Optional minimap** on the navigation card (the owner's request), off by default, toggled from the phone app or on Glass.
- Navigation card driven by a phone navigation source (an app with a navigation API first, Google Maps' notification as best-effort fallback), readable at a glance, updating by itself, working with Android Auto connected.
- Phone notifications on Glass, dismissed in sync, with dictated replies through `RemoteInput`.
- Automatic media sync under charging + Wi-Fi, plus on-demand transfer.
- The native features the firmware already offers through the original companion protocol: connection, battery and device info, time zone, Wi-Fi setup, phone GPS for Glass, screencast, remote control.
- Architecture ready for a later iOS client (transport separated from feature logic and message model).

### Non-goals (for now)
- Glassware store, cloud contacts, remote factory reset, media remote control, Enterprise Edition, the iOS app itself (only `docs/ios-plan.md` later, on request).
- Re-enabling native Glass Maps turn-by-turn (`com.google.glass.maps` is not in XE24 and needed Google routing).

### Anti-goals (a failure even if it works)
- **We drain the Glass battery noticeably** (keeping radios, screen, CPU or wakelocks busy without need).
- **The navigation card lags the maneuver** so it is shown after the turn.
- Anything that needs a Google account, Google servers for Glass, or root on Glass.
- Committing Google binaries or code (clean-room rule).

## 2. Options evaluated

| | A. Original protocol only | B. Own protocol + bridge app only | **C. Hybrid (chosen by the owner)** |
|---|---|---|---|
| App on Glass | none | required (sideload via adb) | optional at first, required for nav/replies/media |
| Native timeline, screencast, touch injection | ✅ | ❌ (no system permissions on API 19) | ✅ via A |
| Navigation card from Maps notification | ⚠️ only as plain timeline cards | ✅ | ✅ via B |
| Notifications with actions + dictated replies | ⚠️ timeline cards, no reply path we control | ✅ | ✅ via B, A fallback |
| Media auto-sync on Wi-Fi + charging | ❌ (needs a process on Glass) | ✅ | ✅ via B |
| Wi-Fi setup, battery, GPS, time zone | ✅ | ✅ (more code) | ✅ via A |
| Path to iOS | ❌ (no RFCOMM on iOS) | ✅ (BLE/Wi-Fi) | ✅ (B part) |
| Testable without hardware | simulated peer only | simulated peer + AVD | both |

**Recommendation and decision: C.** The original protocol (A) delivers the native features with nothing installed on Glass; the bridge app (B) covers what the firmware cannot do (the navigation card, notifications with replies, media sync), and is the base for iOS. Both share one message model on the phone and one feature-logic layer.

## 3. System overview

```
 Android phone                                                  Glass XE24
┌────────────────────────────────────────────┐   RFCOMM secure   ┌───────────────────────────────┐
│ android/ app (Kotlin)                      │  F15CC914-… (A)   │ GlassBluetooth companion      │
│  ├─ NotificationListenerService ───┐       │ ◀───────────────▶ │ server (firmware, untouched)  │
│  ├─ Location, Wi-Fi, UI            │       │                   └───────────────────────────────┘
│  └─ ForegroundService (connectedDevice,    │
│      location)                     ▼       │   Bridge link (B)  ┌───────────────────────────────┐
│ core/ (Kotlin, pure JVM)                   │   RFCOMM own UUID  │ glass-xe/ bridge app (Java,   │
│  ├─ features: nav, notif, media, info, …   │ ◀───────────────▶ │ API 19 + GDK)                 │
│  ├─ CompanionLink  (A: Envelope protocol)  │   or TCP over LAN  │  ├─ nav LiveCard              │
│  ├─ BridgeLink     (B: openmyglass.v1)     │                    │  ├─ notification cards        │
│  └─ Transport: RFCOMM | TCP | (BLE later)  │ ◀── HTTP/TCP ───── │  ├─ dictation recorder        │
└────────────────────────────────────────────┘   media over Wi-Fi │  └─ media sync service        │
                                                                  └───────────────────────────────┘
```

Two independent links to the same Glass:
- **CompanionLink (A):** the phone is an RFCOMM client of the firmware's `Companion` service and speaks the original `Envelope` protocol ([`protocol/`](protocol/)). The firmware accepts one connection at a time, and only our app holds it.
- **BridgeLink (B):** the bridge app on Glass is an RFCOMM server on **our own UUID** (and optionally a TCP server on the LAN), and speaks our protocol `openmyglass.v1`. It is used only when the bridge app is installed.

Every feature works through A, B, or both; section 5 lists which.

## 4. Repository and modules

| Path | Content | Language / target |
|---|---|---|
| `protocol/companion/` | the subset of the original schema we use, copied from `docs/protocol/proto/glass-xe24/` (only messages we send or parse) | proto2 |
| `protocol/openmyglass/v1/` | our own bridge protocol | proto (version and codegen: builder's call, see §8) |
| `core/` | transport-independent logic: `Transport` interface, connection state machine, `CompanionLink`, `BridgeLink`, feature services, Maps-notification parser | Kotlin, pure JVM library (no Android imports), unit-tested on the JVM |
| `android/` | phone app: services, permissions, UI, Android adapters for `core` (Bluetooth sockets, NotificationListener, location, MediaStore, speech) | Kotlin, minSdk **29**, current targetSdk (my call, see §8) |
| `glass-xe/` | bridge app for XE24 | Java, `minSdk = targetSdk = 19`, GDK `compileOnly` stub (AnotherGlass approach) |
| `tools/sim-glass/` | simulated Glass peer speaking **both** protocols over TCP, for hardware-free tests | JVM (Kotlin), reuses `core`/`protocol` |
| `tools/` (existing) | reverse-engineering scripts, clean-room guard; later adb, logcat and HCI-snoop scripts | bash/Python |

`core` must not depend on Android so that the same logic is testable on the JVM and its structure can be mirrored in Swift for iOS.

### Transport abstraction

```kotlin
interface Transport {            // one bidirectional byte stream to the glasses
    suspend fun connect()
    val input: ByteReadSource    // exact types: builder's call
    val output: ByteWriteSink
    fun close()
    val kind: TransportKind      // RFCOMM, TCP, BLE (later)
}
```

- `CompanionLink` and `BridgeLink` each frame messages as **varint-length-delimited protobuf** (the same framing as the original protocol, [`protocol/transport.md`](protocol/transport.md)) on top of any `Transport`.
- Android implementations: `RfcommTransport(device, uuid)`; `TcpTransport(host, port)` for the simulator and for the bridge app over the LAN.
- A connection state machine in `core` (disconnected → connecting → handshaking → connected → backoff) drives reconnection for both links. Reconnect triggers: Bluetooth bond/ACL events, the `PingCompanion` socket (`DC935393-…`) for the firmware's ping, app start, and exponential backoff with a cap.

## 5. Features

### 5.1 Connection and device info — A
- Handshake as described in [`protocol/handshake.md`](protocol/handshake.md): read Glass's `Envelope`, reply with `version` 6.10, `androidDeviceInfo`, `companionDeviceInfoC2G`, `companionFeatureInfoC2G`, `timezoneC2G`.
- Battery, storage, name, software version through `glassInfoRequestC2G`/`glassInfoResponseG2C`, on connect and on demand. No periodic polling (battery anti-goal).

### 5.2 Navigation — B (A fallback)
- **Sources** behind one `NavigationSource` interface in `core`, producing the same `NavUpdate`:
  > Measured 2026-10-19, see [`navigation-sources.md`](navigation-sources.md): OsmAnd's AIDL gives maneuver type and distance but no street/ETA, while its notification carries street, remaining distance/time and ETA. The user must enable OpenMyGlass in OsmAnd once. Google Maps' notification gives distance, road and ETA, and the maneuver only as an icon; Maps needs notification permission.
  1. **Primary: a navigation app with a public API.** The candidate is **OsmAnd** (open source, offline maps, Android Auto support), which offers an AIDL API with navigation-update callbacks (next turn, distance, street) **[unverified: exact fields, behavior with Android Auto connected, minimum OsmAnd version]**. Its offline maps also fit the cloud-free goal. The first navigation task checks this API and picks the app; if OsmAnd does not fit, the builder evaluates other apps with a public navigation API before falling back.
  2. **Fallback: Google Maps' ongoing navigation notification** through `NotificationListenerService` (`com.google.android.apps.maps`), best effort. Which fields it exposes is **unknown**; a capture tool in the phone app dumps extras, actions and icons into a fixture corpus (Italian and English, by car and on foot, with and without Android Auto). If the text fields are not enough, the builder may send the notification's maneuver icon and text that are available, but does not scrape other apps' UI.
- **Android Auto:** both sources must keep producing updates while the phone is projecting to the car. This is part of the acceptance tests for the source (emulator with the Android Auto desktop head unit, then a real car).
- **To Glass:** `NavUpdate{maneuver text, distance, street, eta, icon_id}`. Icons are sent once and cached by hash on Glass; updates carry only changed fields. Updates are sent when the source reports a change, with no polling.
- **On Glass:** a GDK **LiveCard** (published to the timeline as "now") renders the maneuver at 640×360. The screen is woken (wakelock with `ACQUIRE_CAUSES_WAKEUP`) only when the maneuver gets close (a distance threshold, to be tuned) or changes, then turns off by itself. The card is removed when navigation ends.
- **Latency budget:** from source update to card update **under 1 s** on Bluetooth (target to verify on hardware).
- **Minimap (optional):**
  - **Rendered on the phone, never on Glass** (battery): a small north-up or heading-up map image around the current position, with the route line when the source provides route geometry. It is sent as `MapFrame{image, bearing, scale}` and shown in a corner of the navigation card, or full card on a tap.
  - Renderer: an offline map renderer on the phone. On Android, MapLibre with offline OSM tiles, or OsmAnd if its API can provide a map snapshot **[unverified]**. On iOS, `MKMapSnapshotter`. The builder chooses the renderer; tiles come from offline/OSM data, not Google servers for Glass.
  - Fallback renderer on Android: cropping the navigation app's screen with MediaProjection (the brief's experimental mirroring option), only while the owner enables it.
  - **Rate and size:** low resolution (around 200–240 px), compressed, sent only when it changes meaningfully. That means every few seconds while moving and more often close to a maneuver; exact rates are tuned to stay inside BLE throughput for iOS. The minimap stops when Glass's screen is off.
  - Off by default; it costs Bluetooth airtime and Glass screen time.
- **A fallback** (no bridge app installed): a single native timeline item updated in place (`timelineItem`, field 5). It is coarser (no wake control) and only a fallback.

### 5.3 Notifications and dictated replies — B (A fallback)
- **Phone:** `NotificationListenerService` reads title, text, app, icon, category, priority, actions and `RemoteInput`s. Per-app filters, silent notifications, grouping. Updates and removals are kept in sync with Glass.
- **Glass:** a notification card per notification, or bundle per app (layout: builder's call within GDK). Tap opens actions: **Reply**, the notification's own actions, Dismiss.
- **Dictated reply:**
  1. Glass records the reply with the microphone and streams the audio to the phone over the BridgeLink.
  2. **The phone transcribes offline** (the owner's requirement: no recognition on Glass, no cloud speech service). A `SpeechToText` interface in the phone app has two implementations:
     - Android's on-device recognizer fed with the Glass audio (`SpeechRecognizer.createOnDeviceSpeechRecognizer` with an audio-source extra, API 33+), used when the phone supports it for the chosen language **[unverified for Italian on the owner's phone]**;
     - otherwise a **bundled offline engine** (e.g. Vosk or whisper.cpp with an Italian model, downloaded on first use). Engine choice and model size are the builder's call, weighed against phone storage and latency.
  3. The transcription goes back to Glass for a confirm/cancel step.
  4. On confirm, the phone fires the notification's `RemoteInput` action.
  The audio format and encoding are the builder's call: smallest radio and CPU cost on Glass that the phone recognizer accepts.
- **A fallback:** notifications as native timeline items (field 5), read-only. Field 45 (`notificationC2G`) is **not used**: XE24 has no consumer for it.

### 5.4 Photos and videos — B
- The bridge app watches Glass's camera folder (path: builder verifies on XE24).
- **Auto-sync:** only when Glass is **charging and on Wi-Fi**, and the phone is reachable on the same LAN (discovery via Android NSD/mDNS, or the address announced over the BridgeLink). Transfer over Wi-Fi (HTTP or TCP: builder's call), resumable, verified by hash, then saved into the phone's gallery (`MediaStore`). Files on Glass are kept unless the owner enables deletion after sync.
- **On demand:** the phone app lists media on Glass (metadata over BridgeLink) and pulls selected items, over Wi-Fi when available, otherwise over Bluetooth with a warning about speed.
- The original `photoG2C` (field 28) thumbnails are not needed.

### 5.5 Location — A
- Glass requests location with `locationRequestG2C` and the phone streams `locationMessageC2G` while requested; the phone stops its own GPS when Glass stops asking (battery).
- **[unverified]** whether third-party GDK apps on Glass see these locations through Android's `LocationManager`. The bridge app does not need them for navigation (Maps on the phone does the routing). Exposing a mock location provider for other Glass apps is out of scope unless needed later.

### 5.6 Wi-Fi setup, time zone, screencast, remote control, text input — A
- Wi-Fi: `setupWifiC2G` with the `SSID\n{WPA,WEP,NONE}\nKEY` string from a phone-side picker; password prompts via `inputBoxRequestG2C`/`inputBoxResponseC2G`.
- Time zone: `timezoneC2G` on connect and on change.
- Screencast: `screenshot` start/stop, JPEG 640×360 frames shown on the phone; `motionC2G` for touch control from the phone. Only while the screen is open in the phone app.
- Remote keyboard: input box / open-ended input messages.

### 5.7 Lower priority
- Internet sharing for Glass (Glass Proxy channel or standard tethering), SMS through the native timeline, wallpaper: later phases, after analysis.

## 6. Battery rules (Glass)
- Bridge app: no polling loops and no permanent wakelocks; an event-driven service that the phone wakes up through the BridgeLink.
- Screen wakes only for navigation maneuvers close by and new notifications the owner allowed to wake (setting).
- Wi-Fi media transfer only under charging + Wi-Fi.
- Audio recording only while the owner is dictating.
- Phone GPS only while Glass asks for location or navigation is active.
- A battery test on hardware (one hour of navigation, measured drain against stock behavior) is part of the acceptance criteria once Glass is available.

## 7. Development order (revised for the owner's priorities)

1. `core` skeleton: `Transport`, framing, connection state machine, `sim-glass` peer over TCP.
2. CompanionLink (A): handshake, device info and battery, time zone, reconnection, against `sim-glass`.
3. Navigation sources: OsmAnd API check and adapter; Google Maps notification capture tool, fixture corpus and parser; Android Auto checks on the emulator.
4. Bridge app skeleton (`glass-xe`, GDK toolchain) + BridgeLink + **navigation card**, then the optional **minimap**.
5. Notifications: read and dismiss, then actions, then dictated replies.
6. Location (A).
7. Media sync (auto + on demand).
8. Wi-Fi setup, screencast, remote control, text input (A).
9. Settings and app management (sideload helper).

All steps run without hardware (sim-glass, emulators); each step adds its device-only checks to the hardware checklist in §11.

## 8. Decisions made by the assistant (owner may override)
- Phone app **Kotlin**, minSdk **29** (Android 10), current targetSdk. The Android on-device recognizer path needs API 33+; on older phones the bundled offline engine is used. The owner's phone model is not known yet.
- **Distribution:** sideload and GitHub releases, kept F-Droid-compatible; **no Play Store**. This keeps notification access, `connectedDevice`/`location` foreground services and background Bluetooth free of Play policy limits. The Glass bridge app is sideloaded through adb in any case.
- Bridge app in **Java** for API 19 with a GDK `compileOnly` stub, as AnotherGlass does. Whether `gdk.jar` can be committed is checked before Phase 4; otherwise the build takes it from a user-provided path.
- The same varint-delimited protobuf framing for both links.
- Navigation first, before notifications (the brief had it fifth).
- Both links can be open at once; the phone app owns the CompanionLink exclusively.

## 9. Left to the builder
- Protobuf codegen and runtime that work on **API 19** for the bridge app and on the JVM/Android for `core` (e.g. protobuf-javalite or Wire), as long as the wire format stays standard proto.
- Exact `Transport` stream types, coroutine structure, DI.
- GDK card layouts within the 640×360 Glass style.
- Audio encoding for dictation, the media transfer protocol (HTTP or raw TCP), LAN discovery method.
- The bridge app's RFCOMM UUID (a new random UUID, documented in `protocol/openmyglass/`).

## 10. iOS readiness

iOS cannot use the CompanionLink (no Bluetooth Classic RFCOMM to non-MFi devices), so on iOS **everything goes through the bridge app**. The architecture keeps that path open:

- **One bridge protocol for both phones:** `openmyglass.v1` is the only contract the bridge app knows. Android and iOS clients send the same messages (`NavUpdate`, `MapFrame`, notifications, audio, media), so the Glass app does not change.
- **Transports:** the bridge app gets a **BLE peripheral/central transport** (GATT service with a write characteristic and a notify characteristic carrying the same varint-framed messages) and keeps TCP over the LAN for media. IrisGlass shows that XE24 can do BLE with an iPhone ([`prior-art.md`](prior-art.md)). Message sizes are designed for BLE: icons cached by hash, minimap frames small and rate-limited, audio chunked.
- **Feature logic:** `core` stays free of Android APIs so that its state machines and message handling can be mirrored in Swift (or shared later through Kotlin Multiplatform; builder's call when the iOS phase starts).
- **What changes on iOS** (detail for `docs/ios-plan.md`, Phase 5):
  - Navigation source: iOS does not expose other apps' navigation, so the iOS app does its own routing (MapKit `MKDirections`) and uses `MKMapSnapshotter` for the minimap.
  - Notifications: read on Glass through **ANCS**. The bridge app talks to the iPhone's notification service directly over BLE, with positive/negative actions only and no free-text replies.
  - Background limits restrict what works with the iOS app closed.

## 11. Testing
- **Unit (JVM):** framing, `Envelope` encoding against the reconstructed schema, connection state machine, Maps notification parser on the captured fixture corpus, `NavUpdate` diffing.
- **Integration without hardware:** phone app (debug build with `TcpTransport`) on an Android emulator ↔ `sim-glass` through `adb reverse`; bridge app logic on an arm64 AVD with GDK stubs (see [`emulator-feasibility.md`](emulator-feasibility.md)).
- **Hardware checklist** (kept in `docs/progress.md`, filled as features land): XE24 accepts our handshake; CompanionLink and BridgeLink open at the same time; LiveCard wake behavior; navigation latency under 1 s; camera folder path; adb sideloading on the owner's Glass; battery drain (§6).
- **Hardware (when available):** adb scripts to install the bridge app, read `logcat` on both devices, enable and pull the phone's HCI snoop log, compare against [`protocol/`](protocol/); battery measurement (§6).
