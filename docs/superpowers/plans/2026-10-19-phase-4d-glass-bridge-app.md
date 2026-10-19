# Phase 4d: Glass bridge app with the navigation card — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An OpenMyGlass bridge app for Glass XE24 that accepts the phone's bridge connection, keeps the navigation state and icon cache, and shows the navigation card. It is verified end to end on the Android emulator: the phone app and an `emulator` build of the Glass app run on the same AVD, with TCP over `127.0.0.1`.

**Architecture:** A new Android app module `:glass-xe:app` (Java; minSdk/targetSdk 19) with two product flavors:
- `glass`: GDK LiveCard, `compileOnly` against the GDK;
- `emulator`: a plain 640×360 Activity, no GDK.

The bridge logic is plain Java in the flavor-independent source set and is unit-tested on the JVM: TCP server, protocol handling, navigation state, icon cache. A `:glass-xe:gdk-stubs` module provides original compile-only stubs written from the **public GDK reference documentation**. CI-like builds compile the `glass` flavor against both the stubs and Google's real `gdk.jar`, to prove compatibility. Bluetooth RFCOMM and the phone-side foreground service are plan 4e.

**Tech Stack:** Java 17 source level compiled for API 19 (D8 desugaring, as AnotherGlass does), AGP 9.4.x, protobuf-javalite 4.36.x (same `:protocol` module as the phone), JUnit 4.

**Spec:** [`docs/architecture.md`](../../architecture.md) §3, §4, §5.2, §6, §8, §10, §11; research in `_re/research/gdk-and-dhu.md` (local).

## Global Constraints

- Glass target: XE24, Android 4.4.4, **API 19**; no ARM32 native code in our app.
- **GDK license:** Google's `gdk.jar` was installed with the owner's acceptance of the GDK license (`add-ons;addon-google_gdk-google-19`, sha1 `24f09eb48193e5fb09d879b9d36ce8b19948ba04`). It must **never** be committed, copied into the repo, decompiled or disassembled (license §3.3). The build reads it from `$ANDROID_HOME/add-ons/addon-google_gdk-google-19/libs/gdk.jar` or `-PgdkJar=<path>`.
- Stubs are written **only from the public GDK reference docs** (developers.google.com/glass/develop/gdk/reference, CC BY 4.0 docs). Every stub class cites the reference page it comes from. Constant values come from the docs; where the docs do not state a value, the stub omits the constant.
- Battery rules (§6): event-driven, no polling, wake the screen only near a maneuver, release wakelocks with a timeout.
- Bridge protocol `openmyglass.v1` and framing as on the phone; Glass answers the phone's Hello.
- English only; no Claude attribution in commits or PRs; `bash tools/check-clean-room.sh` before every commit (extend it to reject `gdk.jar` anywhere in the tree).

## Review Focus

1. **Phone reconnects while a card is shown** (new session without NavEnded): the old state is replaced or cleared, never duplicated → test in Task 3.
2. **Icon arrives after the NavState that references it** (or never): the card renders without it, requests it once, and updates when it arrives → test in Task 3.
3. **Malformed frame or oversized length from the phone**: the session closes, the server keeps accepting → test in Task 2.
4. **Two phones connect** (second client while one is active): the newest wins or the second is refused, deterministically and without crashing → test in Task 2.
5. **Navigation ends while the screen is off**: the card is removed and the wakelock released → test in Task 4 (logic) and hardware checklist (real LiveCard).

---

## File Structure

| File | Responsibility |
|---|---|
| `glass-xe/gdk-stubs/` (`:glass-xe:gdk-stubs`) | original compile-only stubs of the GDK classes we use |
| `glass-xe/app/build.gradle.kts` (`:glass-xe:app`) | flavors `glass` / `emulator`, GDK jar or stubs selection |
| `glass-xe/app/src/main/java/org/openmyglass/glass/bridge/BridgeServer.java` | TCP accept loop, one session at a time |
| `…/bridge/BridgeSession.java` | framing, Hello, dispatch of frames |
| `…/nav/NavCardState.java` | current navigation state + icon cache, change listener |
| `…/nav/IconCache.java` | LRU of PNG bytes by SHA-256, tracks requested hashes |
| `…/nav/ManeuverArrows.java` | maneuver → our own vector arrow drawable |
| `glass-xe/app/src/emulator/java/…/EmulatorCardActivity.java` | 640×360 card for the emulator |
| `glass-xe/app/src/glass/java/…/NavLiveCardService.java` | LiveCard publishing, wake near maneuver |
| `glass-xe/app/src/main/res/layout/nav_card.xml` | card layout shared by both renderers (RemoteViews-compatible) |
| `android/app/.../DebugActivity.kt` | default bridge address for the same-device test (`127.0.0.1:7825`) |

---

### Task 1: Module skeletons, GDK selection and stubs

**Files:** Create `glass-xe/gdk-stubs/build.gradle.kts` (java-library, Java 8 target) with stubs for `com.google.android.glass.timeline.LiveCard` (+`PublishMode`), `com.google.android.glass.widget.CardBuilder` (+`Layout`) — only what the plan uses — each method body `throw new RuntimeException("Stub!")`, each class Javadoc citing its reference URL; `glass-xe/app/build.gradle.kts`; `glass-xe/app/src/main/AndroidManifest.xml`; flavor manifests; Modify `settings.gradle.kts`, `gradle/libs.versions.toml`, `tools/check-clean-room.sh` (+ test case "gdk.jar rejected"), `glass-xe/README.md`, `.gitignore`.

**Interfaces — Produces:** application id `org.openmyglass.glass` (flavor suffixes `.emulator`), Gradle property `gdkStubs=true|false` (default `false` = real jar; `true` = `:glass-xe:gdk-stubs`), tasks `:glass-xe:app:assembleGlassDebug`, `:glass-xe:app:assembleEmulatorDebug`.

- [ ] **Step 1: Failing test** — add `"gdk.jar rejected"` to `tools/tests/test-check-clean-room.sh` (a tracked `libs/gdk.jar` must fail) → run → FAIL.
- [ ] **Step 2:** Extend the guard (reject any tracked path ending in `gdk.jar`) → test PASS.
- [ ] **Step 3:** Create the modules; the `glass` flavor fails with a clear message if the jar is missing and `gdkStubs` is false.
- [ ] **Step 4: Verify** — `./gradlew :glass-xe:app:assembleEmulatorDebug :glass-xe:app:assembleGlassDebug` and `./gradlew -PgdkStubs=true :glass-xe:app:assembleGlassDebug` → both BUILD SUCCESSFUL; `aapt dump badging` shows `sdkVersion:'19'`.
- [ ] **Step 5: Commit** `glass-xe: app module, flavors, GDK selection and original stubs`.

### Task 2: BridgeServer and BridgeSession (Java, JVM-tested)

**Files:** Create `BridgeServer.java`, `BridgeSession.java`; Test `glass-xe/app/src/test/java/org/openmyglass/glass/bridge/BridgeServerTest.java` (connects with `core`'s `BridgeLink` over TCP; `testImplementation(project(":core"))`).

**Interfaces — Produces:** `final class BridgeServer implements Closeable { BridgeServer(int port, FrameHandler handler); int start(); void close(); }`, `interface FrameHandler { void onSessionStarted(Sender sender); void onFrame(Bridge.Frame frame); void onSessionEnded(); }`, `interface Sender { void send(Bridge.Frame frame) throws IOException; }`. One session at a time; **a new connection replaces the active one** (the phone just reconnected; the old socket is closed and `onSessionEnded` fires before `onSessionStarted`).
- [ ] **Failing tests:** `answers hello with capabilities nav and icons`; `delivers frames in order`; `malformed frame ends the session but the server keeps accepting` (*Review Focus 3*); `a new connection replaces the active session` (*Review Focus 4*).
- [ ] RED → implement (blocking I/O on two threads: accept, session) → GREEN → commit `glass-xe: bridge server`.

### Task 3: NavCardState and IconCache

**Files:** Create `NavCardState.java`, `IconCache.java`; Tests `NavCardStateTest.java`, `IconCacheTest.java`.

**Interfaces — Produces:** `final class NavCardState implements FrameHandler { interface Listener { void onChanged(Card card); void onEnded(); } static final class Card { maneuver, distanceM (Integer), instruction, street, eta, source, iconPng (byte[] or null) } NavCardState(IconCache cache, Listener listener); }` — sends `IconRequest` through the session's `Sender` for an unknown hash, once; on `Icon` updates the card if it references that hash; `onSessionStarted` keeps the last card (phone resends state) and `onSessionEnded` keeps it for 30 s then ends it unless a new state arrives (timer injected for tests). `final class IconCache { IconCache(int maxEntries); byte[] get(String hex); void put(String hex, byte[] png); boolean markRequested(String hex); }`.
- [ ] **Failing tests:** `nav state produces a card`; `nav ended removes the card`; `unknown icon is requested once and the card updates when it arrives` (*Review Focus 2*); `reconnect without nav ended replaces the state` (*Review Focus 1*); `session loss ends the card after the grace period`; `cache evicts the least recently used`.
- [ ] RED → implement → GREEN → commit `glass-xe: navigation card state and icon cache`.

### Task 4: Renderers (emulator Activity, Glass LiveCard)

**Files:** Create `nav_card.xml` (LinearLayout/ImageView/TextView only: RemoteViews-safe), our own arrow vector drawables per `Maneuver` (simple paths, original), `ManeuverArrows.java`, `CardBinder.java` (fills a `RemoteViews` from `Card`), `EmulatorCardActivity.java` + `EmulatorBridgeService.java` (emulator flavor: starts `BridgeServer` on port 7825, shows the card in a 640×360 window), `NavLiveCardService.java` (glass flavor: `LiveCard` with `setViews`, `PublishMode.REVEAL` on first state, `unpublish` on end, wake lock `SCREEN_BRIGHT_WAKE_LOCK | ACQUIRE_CAUSES_WAKEUP` with 5 s timeout when `distanceM` crosses below 200 m or the maneuver changes); Test `WakePolicyTest.java` for the pure decision `boolean shouldWake(Card previous, Card next)` (*Review Focus 5*: ended → release).
- [ ] **Failing tests:** `wakes when crossing 200 m`, `wakes on a new maneuver`, `does not wake on small distance changes`, `does not wake when navigation ends`.
- [ ] RED → implement → GREEN; `assembleGlassDebug` with real jar and with stubs → SUCCESS → commit `glass-xe: navigation card renderers`.

### Task 5: End to end on the emulator

**Files:** Modify `android/app/.../DebugActivity.kt` (preset buttons `127.0.0.1:7825` same-device, `10.0.2.2:7824` sim); Create `docs/glass-app.md` (build, flavors, GDK license note, stubs provenance, install with adb); Modify `docs/progress.md` (hardware checklist: LiveCard publishes on XE24; wake behavior; RFCOMM in 4e).
- [ ] **Step 1:** Install `glass-xe/app` emulator flavor and the phone app on the running Android 36 AVD; start the emulator card; connect the phone app to `127.0.0.1:7825`.
- [ ] **Step 2:** Run an OsmAnd and a Google Maps navigation with `simulate-route.py`. Expected: the card shows the maneuver arrow (ours for OsmAnd, Maps icon for Google Maps), distance, street/instruction, ETA; disappears on stop. Take screenshots (kept in `_re/emulator/`, not committed).
- [ ] **Step 3: Commit** `docs: Glass bridge app, emulator end-to-end`.
