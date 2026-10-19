# Project: OpenMyGlass — reverse engineering MyGlass and an open-source companion without the Google cloud

> English translation of the project owner's brief ([original in Italian](project-prompt.it.md)). If the two differ, the original wins.

## Role and goal

You are a senior reverse-engineering and mobile engineer. Your job is to:

1. Retrieve and analyze the **MyGlass** Android app (the official Google Glass companion) and its counterpart on the glasses in the **XE24** firmware of Glass Explorer Edition.
2. Map **all** the native features MyGlass offered and how it communicated with the glasses.
3. Design and implement, in a new repo, an open-source companion, **Android first**, with an architecture ready for a **later iOS version**.

Functional target:
- **Navigation**: **Google Maps** turn-by-turn instructions from the phone shown on the glasses.
- **Full phone notifications** on the glasses, with actions and replies where possible.
- **Glass management** as MyGlass did: Wi-Fi setup, screencast, battery status, settings, contacts, photo/video sync, app (Glassware) management, shared GPS location.
- **Zero dependencies on the Google cloud for Glass**: no Mirror API, no Google account on the glasses, no MyGlass server services. Everything local between phone and glasses. (Google apps on the phone, such as Google Maps, may be used as data sources.)

Target device: **Glass Explorer Edition XE-C with XE24 firmware** (Android 4.4.2 / API 19, GDK). Enterprise Edition 2 maybe in the future, not now.

## Step 0 — Tool inventory

First, list the plugins, MCP servers, skills and CLI tools available in this session (e.g. jadx, apktool, Ghidra or a Ghidra MCP, adb, gradle, Android SDK, emulators, web tools). Pick the right ones for each phase and, if something essential is missing, install it or tell me exactly what is missing. Do not proceed assuming tools you have not verified.

## Phase 1 — Binary retrieval

- Retrieve the MyGlass Android APK. Verify the package name (should be `com.google.glass.companion`, but check) and pick **the latest version** available (2017 or later); if useful, also a 2014 version for comparison. Use reliable sources, verify signature and hash, and record the source in the documentation.
- Retrieve the official **XE24** system image of Glass Explorer Edition from the Google for Developers "System and Kernel Downloads" page. Extract the system apps and services implementing the glasses side of the companion protocol (Bluetooth services, setup, GlassHome/timeline, navigation, notifications, screencast, media sync).
- **Do not commit** APKs, system images or decompiled code. Keep them in a local folder excluded by `.gitignore` (e.g. `_re/`).

## Phase 2 — Reverse engineering and mapping

Decompile (jadx/apktool) and analyze both sides. Produce documentation, not copies of code:

1. **Feature inventory** (`docs/feature-inventory.md`): for each MyGlass feature, state what it did, which classes implement it on both sides, which messages it exchanges, whether it depended on the Google cloud, and how we will rebuild it (or why not).
2. **Protocol specification** (`docs/protocol/`): transport (expected: Bluetooth Classic RFCOMM, to verify), UUID/SDP, pairing, handshake and authentication, framing, message definitions (probably protobuf: rebuild the `.proto` from the generated classes), sequences for each feature, error handling, keep-alive and reconnection. State in particular whether the handshake requires Google tokens or accounts, and how XE24 firmware behaves after the final update that removed the account requirement.
3. **Glass internal APIs** (`docs/glass-internals.md`): intents, broadcasts, content providers and system services the companion used to drive native features (navigation, timeline, notifications, location, screencast). State what a third-party app can reach with GDK/Android APIs and what needs system permissions or root.
4. **Cloud dependencies** (`docs/cloud-dependencies.md`): everything that went through Google servers (e.g. route computation for native navigation, timeline sync, Glassware installation) and how to replace it locally.

Also study existing projects and reuse what can be reused, respecting licenses:
- `thorikawa/GlassBluetoothProtocol` and `thorikawa/GlassRemote`: reverse engineering of the Bluetooth companion protocol.
- `thalmiclabs/myo-glass-companion`: using the companion protocol from an Android phone to control the native Glass UI.
- `inckie/AnotherGlass`: open-source Android companion without Google services (GPS, notifications, Wi-Fi), with its own protocol and a glasses-side app.
- IrisGlass (`code.nym.sh/kennethnym/aris-old`, `IrisGlass` folder): XE24 app acting as BLE central towards an iPhone, with a GDK LiveCard. Useful for the future iOS phase.

Summarize in `docs/prior-art.md` what they cover, what still works and what is missing.

**Stop here and show me the feature inventory before proceeding.**

## Phase 3 — Architecture (stop and ask for confirmation)

Write `docs/architecture.md` with the options and a reasoned recommendation. Points to evaluate explicitly:

### Communication strategy on Android
- **A. Original protocol emulation:** the Android companion impersonates MyGlass over Bluetooth Classic RFCOMM and talks to the companion service already in the firmware. Advantage: native Glass features (navigation, timeline, Wi-Fi setup, screencast) potentially **without installing apps on the glasses**. To verify: handshake, possible identity/signature checks, features that depended on cloud responses.
- **B. Own protocol + bridge app on the glasses** (AnotherGlass style): maximum freedom, UI rebuilt with GDK (LiveCard, CardBuilder, gestures, voice trigger).
- **C. Hybrid:** original protocol for what works natively, bridge app for what is missing or depended on the cloud.

### Abstraction for the future iOS app
Cleanly separate the **transport layer** from the **application logic** and the **message model**:
- a `Transport` interface with interchangeable implementations (RFCOMM, BLE, Wi-Fi/TCP);
- a versioned message model (`.proto` or equivalent schema) used by all clients;
- feature logic (notifications, navigation, management) independent of the transport.

Reason: an iOS app **cannot** open Bluetooth Classic RFCOMM connections to non-MFi devices, so the iOS version will use BLE and/or Wi-Fi and a bridge app on the glasses, reusing the same message model.

### Notifications on Android
- `NotificationListenerService` to read notifications from all apps: title, text, app, icon, category, priority, actions.
- Running actions (`PendingIntent`) and **replies** through `RemoteInput`, with voice dictation from the glasses.
- Per-app filters, silent notifications, grouping, update and removal kept in sync with the glasses.

### Google Maps navigation on Android
Evaluate and document these options, in order of preference:
1. **Reading Google Maps' navigation notification** through `NotificationListenerService`: while navigating, Maps posts a persistent notification with next maneuver, distance, street and possibly a maneuver icon. Extract the data (text and icon bitmap) robustly across language and version variants, and send it to the glasses in a navigation card optimized for 640×360. Document the fields actually available.
2. **Feeding Glass's native navigation** through the original protocol, if reverse engineering shows it is possible without cloud-side route computation.
3. **Screen mirroring with MediaProjection**, cropping the map area and scaling it for the glasses. Experimental mode: evaluate latency, Wi-Fi bandwidth and battery usage.

In any case, forward the phone's **GPS location** to the glasses (which have no GPS) as a mock location, so apps on the glasses use it.

**Stop and ask me to confirm the architecture before implementing.**

## Phase 4 — Android implementation

Proposed repo layout (adapt it to the approved architecture):

```
/docs                 reverse-engineering and architecture documentation
/protocol             message definitions (.proto or schema), versioned
/core                 transport-independent feature logic
/android              Android companion app
/glass-xe             GDK bridge app for Explorer XE24 (if planned)
/tools                simulated peers, test harnesses, adb scripts
/ios                  (later phase, not now)
```

Requirements:
- **Android app**: Kotlin, current target SDK, reasonable minSdk (state the choice), foreground service of type `connectedDevice`/`location`, correct runtime permission handling (Bluetooth on Android 12+, notifications, location, notification access), battery optimization exemption on user request, automatic reconnection.
- **Android UI**: pairing and connection status, battery and glasses info, Wi-Fi setup, notification filters, navigation, screencast, synced photo/video gallery, settings, management of apps on the glasses.
- **App on the glasses** (if planned): compatible with API 19 and GDK; pick a toolchain that really builds against the GDK add-on and document it. Persistent service, minimal power usage, UI consistent with the Glass style.
- **Tests without hardware**: simulated peer for both sides, unit tests for serialization, the connection state machine and Google Maps notification parsing (with sample notifications), end-to-end integration tests with an emulator.
- **Tests with hardware**: adb scripts to install, read logs (`logcat`) and capture Bluetooth traffic (the phone's HCI snoop log) to compare against the spec.

Development order:
1. Pairing, connection, handshake, keep-alive.
2. Battery status and device info.
3. Notifications (read, display, remove), then actions and replies.
4. Shared GPS location.
5. Google Maps navigation.
6. Wi-Fi setup.
7. Photo/video sync.
8. Screencast.
9. App management and settings.
10. Experimental MediaProjection mirroring.

## Phase 5 — iOS (only after Android is stable, and only if I ask)

Prepare only `docs/ios-plan.md` with the plan, without implementing until I ask:
- BLE (CoreBluetooth) and/or local Wi-Fi transport to the bridge app on the glasses, reusing `/protocol` and the `/core` logic where possible.
- Notifications through **ANCS** (Apple Notification Center Service) on the glasses side: the iOS app cannot read other apps' notifications; ANCS allows positive/negative actions but not free-text replies.
- Navigation: iOS does not expose Google Maps or Apple Maps instructions to other apps. Options: own routing with MapKit (`MKDirections`, `MKMapSnapshotter`), experimental mirroring with a ReplayKit Broadcast Upload Extension, navigation notifications via ANCS if present.
- iOS background limits and what works with the app closed.

## Rules

- Work in phases and keep `docs/progress.md` updated with status, decisions and open issues.
- Commit often with clear messages. No Google binaries or code in the repo: only documentation and original code (clean-room approach; the spec in `docs/` bridges analysis and implementation).
- When a feature cannot be rebuilt (system permissions, signature, irreplaceable cloud dependency), say so explicitly and propose the best alternative instead of faking it.
- If information is uncertain, verify it in the decompiled code or on the device; do not invent UUIDs, protobuf fields, intents or notification formats.
- Stop and ask for confirmation: after Phase 2 (inventory) and after Phase 3 (architecture).
