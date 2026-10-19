# Phase 4b: navigation sources on the phone — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The phone produces a stream of `NavUpdate`s (next maneuver, distance, optional instruction text) from OsmAnd's public API, and a capture tool records what Google Maps' navigation notification actually contains, so that the fallback parser can be written from real samples.

**Architecture:** `core` gains a transport-free navigation model (`Maneuver`, `NavUpdate`, `NavigationSource`) and the OsmAnd turn-type mapping. A new Android app module hosts two adapters: `OsmAndNavigationSource` (AIDL client of OsmAnd) and a `NotificationListenerService` with a capture/export mode for Google Maps notifications. Sending updates to Glass (bridge app, navigation card, minimap) is the next plan (4c).

**Tech Stack:** Kotlin, Android Gradle Plugin 9.4.x, compileSdk/targetSdk 36, minSdk 29, OsmAnd AIDL API (`net.osmand.aidlapi`), JUnit 5 for `core`, JUnit 4 local unit tests for the Android module.

**Spec:** [`docs/architecture.md`](../../architecture.md) §5.2, §7 step 3, §8; research below.

## Research findings (2026-10-19, from `osmandapp/OsmAnd` master, `OsmAnd-api/`)

- `IOsmAndAidlInterface.registerForNavigationUpdates(ANavigationUpdateParams, IOsmAndAidlCallback)` → `callback.updateNavigationInfo(ADirectionInfo)`. `ADirectionInfo` carries only **`distanceTo`** (int, distance to next turn), **`turnType`** (int) and **`isLeftSide`** (bool). **No street name, no ETA.**
- `registerForVoiceRouterMessages(...)` → `onVoiceRouterNotify(OnVoiceNavigationParams{cmds: List<String>, played: List<String>})`: the voice prompts, which may contain street names depending on voice settings **[unverified format]**.
- `getMapScreenshot(MapScreenshotParams{maxWidth, quality, mapOnly}) → AMapScreenshot{image (JPEG), width, height}` works **only while OsmAnd's map is on screen**, so it cannot feed the minimap with the phone in a pocket (the minimap needs our own renderer; plan 4d).
- Turn types (`OsmAnd-java/.../router/TurnType.java`): `C=1, TL=2, TSLL=3, TSHL=4, TR=5, TSLR=6, TSHR=7, KL=8, KR=9, TU=10, TRU=11, OFFR=12, RNDB=13, RNLB=14`.
- OsmAnd repository license is reported as `NOASSERTION` by GitHub; the code is GPLv3-based. The AIDL client files must be vendored only after the license check in Task 3.

## Global Constraints

- `core` stays pure JVM (no Android imports); Android code lives in `android/app`.
- Phone app: Kotlin, **minSdk 29**, targetSdk 36, sideload/F-Droid distribution (no Play Store constraints).
- Navigation must keep working with **Android Auto connected** (architecture §1, §5.2); Task 6 checks it.
- No polling: sources push updates on change only (battery rule §6).
- Never commit Google binaries or Google bitmaps (Maps maneuver icons): fixtures keep text fields and icon metadata (size, hash) only. `bash tools/check-clean-room.sh` before every commit.
- All repo content in English. **No Claude attribution lines in commits or PRs** (owner's instruction).

## Review Focus

1. **OsmAnd not installed, or uninstalled while navigating**: the source reports "unavailable" and emits `null` (no card), no crash → test in Task 4.
2. **OsmAnd service dies or rebinds** (OsmAnd updated/killed): the source re-registers callbacks after reconnection → test in Task 4.
3. **Turn types outside the known table** (newer OsmAnd versions): mapped to `Maneuver.UNKNOWN`, never an exception → test in Task 2.
4. **Navigation ends or is paused**: `NavigationSource` emits `null` so Glass can remove the card → tests in Tasks 2 and 4.
5. **Notification extras of unexpected types** (Bitmap, CharSequence spans, nested Bundles, RemoteViews) in the capture tool: serialized as typed placeholders, never a crash → test in Task 5.

---

## File Structure

| File | Responsibility |
|---|---|
| `core/src/main/kotlin/org/openmyglass/core/nav/NavUpdate.kt` | `Maneuver`, `NavUpdate`, `NavigationSource` |
| `core/src/main/kotlin/org/openmyglass/core/nav/OsmAndTurnTypes.kt` | OsmAnd `turnType` → `Maneuver` |
| `core/src/main/kotlin/org/openmyglass/core/nav/NavUpdateReducer.kt` | merges direction info and voice text into `NavUpdate`s |
| `android/app/` | Android app module (`:android:app`) |
| `android/app/src/main/aidl/net/osmand/aidlapi/…` + parcelables | vendored OsmAnd AIDL client subset (after license check) |
| `android/app/src/main/kotlin/org/openmyglass/android/nav/OsmAndNavigationSource.kt` | binds OsmAnd, registers callbacks, feeds the reducer |
| `android/app/src/main/kotlin/org/openmyglass/android/notifications/OpenMyGlassNotificationListener.kt` | notification listener with Maps capture mode |
| `android/app/src/main/kotlin/org/openmyglass/android/notifications/NotificationDump.kt` | serializes a notification's extras to JSON-able maps |
| `android/app/src/main/kotlin/org/openmyglass/android/DebugActivity.kt` | debug screen: source status, last `NavUpdate`, capture toggle, export |
| `docs/navigation-sources.md` | research findings, capture results, Android Auto results |

---

### Task 1: Navigation model in core

**Files:**
- Create: `core/src/main/kotlin/org/openmyglass/core/nav/NavUpdate.kt`, `OsmAndTurnTypes.kt`; test `core/src/test/kotlin/org/openmyglass/core/nav/OsmAndTurnTypesTest.kt`

**Interfaces:**
- Produces:
  - `enum class Maneuver { STRAIGHT, LEFT, SLIGHT_LEFT, SHARP_LEFT, RIGHT, SLIGHT_RIGHT, SHARP_RIGHT, KEEP_LEFT, KEEP_RIGHT, U_TURN_LEFT, U_TURN_RIGHT, OFF_ROUTE, ROUNDABOUT, ROUNDABOUT_LEFT, ARRIVE, UNKNOWN }`
  - `data class NavUpdate(val maneuver: Maneuver, val distanceMeters: Int?, val instruction: String?, val street: String?, val eta: String?, val source: String)` — `source` is `"osmand"` or `"gmaps"`.
  - `interface NavigationSource { val name: String; val updates: Flow<NavUpdate?>; val available: StateFlow<Boolean> }` — `null` means "no active navigation".
  - `object OsmAndTurnTypes { fun toManeuver(turnType: Int): Maneuver }`

- [ ] **Step 1: Write the failing tests** — `known turn types map to maneuvers`: `1→STRAIGHT, 2→LEFT, 3→SLIGHT_LEFT, 4→SHARP_LEFT, 5→RIGHT, 6→SLIGHT_RIGHT, 7→SHARP_RIGHT, 8→KEEP_LEFT, 9→KEEP_RIGHT, 10→U_TURN_LEFT, 11→U_TURN_RIGHT, 12→OFF_ROUTE, 13→ROUNDABOUT, 14→ROUNDABOUT_LEFT`; `unknown turn types map to UNKNOWN`: `0, 15, -1, 999 → UNKNOWN` *(Review Focus 3)*.
- [ ] **Step 2:** Run `./gradlew :core:test --tests '*OsmAndTurnTypes*'` — Expected: FAIL (unresolved).
- [ ] **Step 3:** Implement. KDoc on `OsmAndTurnTypes` cites `OsmAnd-java/src/main/java/net/osmand/router/TurnType.java`.
- [ ] **Step 4:** Rerun — Expected: PASS (2 tests).
- [ ] **Step 5: Commit** — `git commit -m "core: navigation model and OsmAnd turn types"`.

---

### Task 2: NavUpdateReducer

**Files:**
- Create: `core/src/main/kotlin/org/openmyglass/core/nav/NavUpdateReducer.kt`; test `NavUpdateReducerTest.kt`

**Interfaces:**
- Consumes: `Maneuver`, `NavUpdate`, `OsmAndTurnTypes` (Task 1).
- Produces: `class NavUpdateReducer(private val source: String) { fun onDirection(distanceTo: Int, turnType: Int): NavUpdate; fun onVoicePlayed(played: List<String>): NavUpdate?; fun onNavigationStopped(): NavUpdate? /* always null */ ; val current: NavUpdate? }`
  - `onDirection` keeps the last voice text as `instruction`; a negative `distanceTo` becomes `distanceMeters = null`.
  - `onVoicePlayed` stores the last non-blank entry as `instruction` and returns the updated `NavUpdate`, or `null` if no direction has arrived yet.
  - `onNavigationStopped` clears all state.

- [ ] **Step 1: Write the failing tests:**
  - `direction produces an update` — `onDirection(250, 2)` → `NavUpdate(LEFT, 250, null, null, null, "osmand")`.
  - `voice text becomes the instruction` — `onDirection(250, 2)`, `onVoicePlayed(listOf("", "Turn left onto Via Roma"))` → `instruction == "Turn left onto Via Roma"`; a following `onDirection(120, 2)` keeps it.
  - `voice before any direction yields nothing` — fresh reducer, `onVoicePlayed(listOf("x"))` → `null`.
  - `negative distance is unknown` — `onDirection(-1, 5).distanceMeters == null`.
  - `stop clears state` — after updates, `onNavigationStopped()` returns `null`, `current == null`, and the next `onVoicePlayed` returns `null` *(Review Focus 4)*.
- [ ] **Step 2:** Run — Expected: FAIL. **Step 3:** Implement. **Step 4:** Run — Expected: PASS (5 tests).
- [ ] **Step 5: Commit** — `git commit -m "core: nav update reducer"`.

---

### Task 3: Android app module and OsmAnd AIDL client files

**Files:**
- Create: `android/app/build.gradle.kts`, `android/app/src/main/AndroidManifest.xml`, `android/app/src/main/kotlin/org/openmyglass/android/DebugActivity.kt` (empty screen with app name for now), vendored `android/app/src/main/aidl/net/osmand/aidlapi/**` and matching parcelable Java sources (only what `registerForNavigationUpdates`, `registerForVoiceRouterMessages` and their callback types need), `android/app/third_party/OSMAND-API-NOTICE.md`
- Modify: `settings.gradle.kts` (include `:android:app`), `gradle/libs.versions.toml` (AGP 9.4.x, AndroidX core/activity, JUnit 4), `android/README.md`, `docs/prior-art.md` (OsmAnd row: license and what is vendored)

**Interfaces:**
- Consumes: `:core`.
- Produces: module `:android:app`, application id `org.openmyglass.android`, namespace `org.openmyglass.android`.

- [ ] **Step 1: License check** — read OsmAnd's `LICENSE` and the headers of the `OsmAnd-api` files. Vendoring is allowed only if they are GPLv3-compatible. Record the outcome, the source commit hash and the file list in `OSMAND-API-NOTICE.md`. If they are not compatible, stop and ask the owner.
- [ ] **Step 2:** Add the module (compileSdk 36, minSdk 29, targetSdk 36, Java/Kotlin target 17, `buildFeatures { aidl = true }`); copy the AIDL subset unchanged with its license headers.
- [ ] **Step 3: Verify the build** — `./gradlew :android:app:assembleDebug` — Expected: BUILD SUCCESSFUL. `bash tools/check-clean-room.sh` — Expected: `clean-room ok`.
- [ ] **Step 4: Commit** — `git commit -m "android: app module and vendored OsmAnd AIDL client"`.

---

### Task 4: OsmAndNavigationSource

**Files:**
- Create: `android/app/src/main/kotlin/org/openmyglass/android/nav/OsmAndNavigationSource.kt`, `android/app/src/main/kotlin/org/openmyglass/android/nav/OsmAndBinder.kt` (thin wrapper around `bindService` so the source can be unit-tested with a fake); test `android/app/src/test/kotlin/org/openmyglass/android/nav/OsmAndNavigationSourceTest.kt`
- Modify: `AndroidManifest.xml` (`<queries>` for OsmAnd packages `net.osmand` and `net.osmand.plus`), `DebugActivity.kt` (shows `available` and the last `NavUpdate`)

**Interfaces:**
- Consumes: `NavigationSource`, `NavUpdate`, `NavUpdateReducer` (Tasks 1–2); AIDL types (Task 3).
- Produces:
  - `interface OsmAndBinder { fun bind(onConnected: (OsmAndApi) -> Unit, onDisconnected: () -> Unit): Boolean; fun unbind() }`
  - `interface OsmAndApi { fun registerNavigation(callback: (distanceTo: Int, turnType: Int) -> Unit): Long; fun registerVoice(callback: (played: List<String>) -> Unit): Long; fun unregister(id: Long) }` — wraps the AIDL interface; the real implementation binds OsmAnd's AIDL service (service intent and package per OsmAnd's API documentation/demo, verified in this step).
  - `class OsmAndNavigationSource(binder: OsmAndBinder, scope: CoroutineScope) : NavigationSource` with `fun start()`, `fun stop()`; `name = "osmand"`.

- [ ] **Step 1: Write the failing tests** (fake `OsmAndBinder`/`OsmAndApi`):
  - `bind failure means unavailable` — `bind` returns `false` → `available.value == false`, `updates` emits `null` *(Review Focus 1)*.
  - `direction callbacks become updates` — connected fake invokes `(300, 5)` → `updates` emits `NavUpdate(RIGHT, 300, …, source = "osmand")`.
  - `service death re-registers after reconnect` — `onDisconnected()` → `available == false` and `updates` emits `null`; a second `onConnected(api2)` registers both callbacks again on `api2` *(Review Focus 2)*.
  - `stop unregisters and unbinds` — `stop()` calls `unregister` for both ids and `unbind()`.
- [ ] **Step 2:** Run `./gradlew :android:app:testDebugUnitTest` — Expected: FAIL.
- [ ] **Step 3:** Implement source and real binder.
- [ ] **Step 4:** Rerun — Expected: PASS (4 tests).
- [ ] **Step 5: Manual check on the emulator** (API 36 arm64 image): install OsmAnd (F-Droid APK), download a small offline map, start a simulated route (OsmAnd developer setting "simulate your position"), open `DebugActivity`. Expected: `available = true` and `NavUpdate`s change as the simulation advances. Record the observed voice `played` strings (language, presence of street names) in `docs/navigation-sources.md`.
- [ ] **Step 6: Commit** — `git commit -m "android: OsmAnd navigation source"`.

---

### Task 5: Notification listener with Maps capture mode

**Files:**
- Create: `NotificationDump.kt`, `OpenMyGlassNotificationListener.kt`; test `android/app/src/test/kotlin/org/openmyglass/android/notifications/NotificationDumpTest.kt`
- Modify: `AndroidManifest.xml` (listener service with `BIND_NOTIFICATION_LISTENER_SERVICE`), `DebugActivity.kt` (button to open notification-access settings, capture toggle for package `com.google.android.apps.maps`, "export" that writes captures to `getExternalFilesDir("captures")`)

**Interfaces:**
- Produces:
  - `object NotificationDump { fun dump(packageName: String, postTime: Long, category: String?, isOngoing: Boolean, extras: Map<String, Any?>, actionTitles: List<String>, hasCustomContentView: Boolean, hasCustomBigView: Boolean): Map<String, Any?> }` — a pure function over already-extracted values so it is unit-testable without Android; it is JSON-serializable.
  - Value rules: `String`/`CharSequence` → its `toString()`; numbers/booleans as is; `Bitmap`/`Icon` → `{"type":"image","width":w,"height":h,"sha256":…}` (extracted by the listener before calling `dump`); nested `Bundle` → nested map; arrays/lists → lists; anything else → `{"type": "<class name>"}`.
- [ ] **Step 1: Write the failing tests:**
  - `strings and numbers pass through` — extras `{"android.title":"200 m", "android.progress": 3}` → same values under `"extras"`.
  - `images become metadata only` — an image placeholder map stays as metadata; no raw bytes in the output *(clean-room constraint)*.
  - `unknown types become typed placeholders` — a value of an unknown class `Foo` → `{"type":"Foo"}`, no exception *(Review Focus 5)*.
  - `nested maps and lists are kept` — `{"a": {"b": listOf("x", 1)}}` round-trips.
  - `custom views are flagged` — `hasCustomContentView = true` appears in the output.
- [ ] **Step 2:** Run `./gradlew :android:app:testDebugUnitTest` — Expected: FAIL. **Step 3:** Implement `NotificationDump` and the listener (captures only while the toggle is on; stores one JSON line per posted/updated notification). **Step 4:** Rerun — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "android: notification listener with Maps capture mode"`.

---

### Task 6: Capture session and findings

**Files:**
- Create: `docs/navigation-sources.md`; `android/app/src/test/resources/maps-notifications/*.jsonl` (text fields and image metadata only)
- Modify: `docs/architecture.md` §5.2 (facts from this plan: OsmAnd fields, screenshot limitation, Maps results), `docs/progress.md` (status, hardware checklist: "navigation updates while Android Auto runs on a real car")

- [ ] **Step 1: Google Maps on the emulator** — Play Store image API 36 arm64 (Maps navigation does not need sign-in; if Maps or Play insists on an account, stop and ask the owner to sign in), set a GPS route in the emulator's extended controls, start Google Maps navigation by car and on foot, in Italian and in English, with the capture toggle on. Export and `adb pull` the captures.
- [ ] **Step 2: Android Auto** — if the Desktop Head Unit (SDK extra) runs on this Mac, repeat one Maps session and one OsmAnd session with DHU connected; otherwise record "not testable on this Mac" and add it to the hardware checklist.
- [ ] **Step 3: Write `docs/navigation-sources.md`** — for each source: fields actually present (with examples), update frequency, behavior with the screen off and with Android Auto, gaps. End with a recommendation for the Maps fallback parser: which extras to read, or "not usable" with the reason.
- [ ] **Step 4: Verify fixtures** — no fixture contains image data: `grep -lE 'iVBOR|/9j/' android/app/src/test/resources/maps-notifications/*.jsonl` prints nothing; `bash tools/check-clean-room.sh` → `clean-room ok`.
- [ ] **Step 5: Commit** — `git commit -m "docs: navigation source capture results"`.
