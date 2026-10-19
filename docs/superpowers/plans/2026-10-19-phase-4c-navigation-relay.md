# Phase 4c: navigation relay to Glass (phone side) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The phone turns OsmAnd and Google Maps navigation into complete `NavUpdate`s and sends them, with maneuver icons, to the Glass bridge app over our own protocol `openmyglass.v1`; verified end to end against a simulated bridge peer.

**Architecture:** Parsers for the two notification formats live in `core` (pure JVM, tested on the captured fixtures). `openmyglass.v1` is a new proto package in `:protocol`, framed like the companion link (varint-delimited). `core` gains `BridgeLink` (handshake + messages) and `NavigationRelay` (picks the active source, deduplicates, sends icons once by hash). The phone app wires sources → relay → `BridgeLink` over `TcpTransport` to `tools/sim-glass` running in bridge mode. The real Glass app (GDK, LiveCard) is plan 4d; Bluetooth RFCOMM for the bridge and the foreground service come with it.

**Tech Stack:** Kotlin/JVM, protobuf-javalite 4.36.x (the same runtime will run on Glass API 19 in 4d), JUnit 5 (`core`), JUnit 4 (`android/app`).

**Spec:** [`docs/architecture.md`](../../architecture.md) §3, §4, §5.2, §6, §10; facts from [`docs/navigation-sources.md`](../../navigation-sources.md) and fixtures in `android/app/src/test/resources/maps-notifications/`.

## Global Constraints

- `core` stays pure JVM; Android code only in `android/app`.
- Bridge framing = varint32 length + protobuf (same as `DelimitedFraming`).
- Battery rules (§6): send only on change; icons sent once per hash; no polling.
- BLE-friendly sizes for the future iOS path (§10): icons ≤ 128×128 PNG; no message > 64 KiB.
- Never commit Google bitmaps; icons travel only at runtime.
- English only; no Claude attribution in commits or PRs.

## Review Focus

1. **Locale variants** of Maps text (`200 m`, `2,8 km`, `700 ft`, `0.3 mi`, empty title with `verso …`, `Rielaborazione percorso...`): parse what is there, never throw; unknown → `null` fields → tests in Task 2.
2. **Both sources active at once** (user starts Maps while OsmAnd navigates): one source wins deterministically (OsmAnd first) and Glass is not flooded with alternating updates → test in Task 5.
3. **Bridge link drops while navigating**: the relay resends the current state (and needed icons) after reconnection → test in Task 5.
4. **Identical consecutive updates** (Maps reposts every second): no duplicate sends → test in Task 5.
5. **Oversized or malformed icon** (huge bitmap, non-PNG): scaled/rejected on the phone, never sent above the size limit → test in Task 6.

---

## File Structure

| File | Responsibility |
|---|---|
| `protocol/openmyglass/v1/bridge.proto` | bridge protocol messages |
| `core/src/main/kotlin/org/openmyglass/core/nav/MapsNotificationParser.kt` | Google Maps notification → `NavUpdate` |
| `core/src/main/kotlin/org/openmyglass/core/nav/OsmAndNotificationParser.kt` | OsmAnd notification → instruction/ETA |
| `core/src/main/kotlin/org/openmyglass/core/nav/Distances.kt` | locale-tolerant distance parsing |
| `core/src/main/kotlin/org/openmyglass/core/bridge/BridgeLink.kt` | bridge handshake and messages over a `Channel` |
| `core/src/main/kotlin/org/openmyglass/core/bridge/NavigationRelay.kt` | source selection, dedup, icon cache, resend on reconnect |
| `tools/sim-glass/src/main/kotlin/org/openmyglass/simglass/SimBridge.kt` | simulated Glass bridge app |
| `android/app/src/main/kotlin/org/openmyglass/android/nav/MapsNavigationSource.kt` | Maps notifications → `NavigationSource` |
| `android/app/src/main/kotlin/org/openmyglass/android/nav/IconEncoder.kt` | icon bitmap → scaled PNG + SHA-256 |

---

### Task 1: Bridge protocol `openmyglass.v1`

**Files:** Create `protocol/openmyglass/v1/bridge.proto`; Modify `protocol/build.gradle.kts` (add proto source dir `openmyglass`), `protocol/README.md`; Test `protocol/src/test/kotlin/org/openmyglass/protocol/BridgeProtoTest.kt`

**Interfaces — Produces** (proto3, package `openmyglass.v1`, `java_package = "org.openmyglass.protocol.v1"`, `java_outer_classname = "Bridge"`):
```proto
message Frame {                       // every frame on the bridge link
  oneof body {
    Hello hello = 1;                  // both directions, first frame each way
    NavState nav = 2;                 // phone -> glass: current navigation state (absent fields = unknown)
    NavEnded nav_ended = 3;           // phone -> glass: remove the card
    Icon icon = 4;                    // phone -> glass: icon bytes for a hash
    IconRequest icon_request = 5;     // glass -> phone: "I don't have this hash"
  }
}
message Hello { uint32 protocol_version = 1; string app_version = 2; repeated string capabilities = 3; }
enum Maneuver { MANEUVER_UNKNOWN = 0; STRAIGHT = 1; LEFT = 2; SLIGHT_LEFT = 3; SHARP_LEFT = 4; RIGHT = 5;
  SLIGHT_RIGHT = 6; SHARP_RIGHT = 7; KEEP_LEFT = 8; KEEP_RIGHT = 9; U_TURN_LEFT = 10; U_TURN_RIGHT = 11;
  OFF_ROUTE = 12; ROUNDABOUT = 13; ROUNDABOUT_LEFT = 14; ARRIVE = 15; }
message NavState { Maneuver maneuver = 1; optional int32 distance_m = 2; optional string instruction = 3;
  optional string street = 4; optional string eta = 5; string source = 6; optional bytes icon_sha256 = 7; }
message NavEnded {}
message Icon { bytes sha256 = 1; bytes png = 2; }
message IconRequest { bytes sha256 = 1; }
```
`protocol_version = 1`.

- [ ] **Step 1: Failing tests** — `frame round-trips nav state` (all NavState fields incl. `icon_sha256` of 32 bytes); `absent optional fields stay absent` (`hasDistanceM() == false` after round-trip of an empty NavState); `maneuver values match core` (`Maneuver.ROUNDABOUT.number == 13`, `ARRIVE == 15`).
- [ ] **Step 2:** `./gradlew :protocol:test` → FAIL. **Step 3:** write proto + build config. **Step 4:** → PASS.
- [ ] **Step 5: Commit** `protocol: openmyglass.v1 bridge messages`.

---

### Task 2: Notification parsers (Maps, OsmAnd)

**Files:** Create `Distances.kt`, `MapsNotificationParser.kt`, `OsmAndNotificationParser.kt` in `core/.../nav/`; Tests `MapsNotificationParserTest.kt`, `OsmAndNotificationParserTest.kt`, `DistancesTest.kt` in `core/src/test/.../nav/`; copy fixtures to `core/src/test/resources/nav-fixtures/` (the three `*.jsonl` from `android/app/src/test/resources/maps-notifications/` plus `gmaps-car-it-icons.jsonl`).

**Interfaces — Produces:**
- `object Distances { fun toMeters(text: String): Int? }` — accepts `m`, `km`, `ft`, `mi`, `yd`; decimal `.` or `,`; normal or non-breaking space; returns `null` if no number+unit.
- `data class NotificationText(val title: String?, val text: String?, val subText: String?, val bigText: String?, val iconSha256: String?)`
- `object MapsNotificationParser { fun parse(n: NotificationText): NavUpdate? }` — `distanceMeters` from title; `street` from text after stripping a leading `verso `/`toward ` and taking the part after ` / ` if present; `instruction` = text when it is a sentence (contains a space and no digits-only road ref), e.g. `Fai una inversione a U`; `eta` = last ` · `-separated part of subText with `ETA`/`Arrivo` removed and trimmed; `maneuver` from `MapsIcons.maneuverFor(iconSha256)` (initially one entry: `13b4feacacd8…` prefix → `U_TURN_LEFT`, from the capture where the text says "Fai una inversione a U"; full hash copied from the fixture); `source = "gmaps"`; returns `null` when title, text and subText are all blank. `iconSha256` is kept on the update via a new field `iconSha256: String?` on `NavUpdate` (add it in this task, default `null`).
- `object OsmAndNotificationParser { data class Extra(val instruction: String?, val eta: String?); fun parse(n: NotificationText): Extra }` — bigText line 1 without its trailing distance (`… Obergass 60 m` → `Turn right and go Obergass`), line 2 split on ` • ` → third element is the ETA (`8:17 PM`).

- [ ] **Step 1: Failing tests** (examples verbatim from the fixtures):
  - `Distances`: `"200 m"→200`, `"2,8 km"→2800`, `"1.3 km"→1300`, `"700 ft"→213`, `"0.3 mi"→483`, `"verso Äulestrasse"→null`, `""→null`.
  - Maps (each fixture line parses without exception — loop over all four files; *Review Focus 1*): `title "200 m", text "28 / Landstrasse", subText "2 min · 1.3 km · 8:33 PM ETA"` → `distance 200, street "Landstrasse", eta "8:33 PM"`; Italian `text "verso Äulestrasse", subText "24 min · 1,6 km · Arrivo 20:56"` → `street "Äulestrasse", eta "20:56", distance null`; `text "Rielaborazione percorso..."` with no title → update with `instruction "Rielaborazione percorso..."`, `distance null`; U-turn icon hash → `U_TURN_LEFT`; all-blank → `null`.
  - OsmAnd: `bigText "Turn right and go Obergass 60 m\n2.9 km • 5 min • 8:17 PM • 0 km/h"` → `Extra("Turn right and go Obergass", "8:17 PM")`; missing bigText → `Extra(null, null)`.
- [ ] **Step 2:** `./gradlew :core:test` → FAIL. **Step 3:** implement. **Step 4:** → PASS.
- [ ] **Step 5: Commit** `core: Maps and OsmAnd notification parsers`.

---

### Task 3: BridgeLink

**Files:** Create `core/src/main/kotlin/org/openmyglass/core/bridge/BridgeLink.kt`; Test `BridgeLinkTest.kt` (reuse the loopback-socket pattern of `FakeGlass`, here as `FakeBridge` sending/reading `Bridge.Frame`).

**Interfaces:**
- Consumes: `Channel`, `DelimitedFraming`, `Bridge.*`.
- Produces: `class BridgeLink(channel: Channel, appVersion: String)` with `suspend fun handshake(timeout: Duration = 10.seconds): Bridge.Hello` (phone sends its Hello first, then reads Glass's — the bridge app is ours, so the phone leads), `suspend fun send(frame: Bridge.Frame)`, `fun incoming(): Flow<Bridge.Frame>`, `fun close()`. Frames above 64 KiB are rejected with `IllegalArgumentException` before writing.
- [ ] **Steps:** failing tests `handshake exchanges hellos`, `handshake times out on a silent peer`, `oversized frame is rejected`, `incoming completes on disconnect` → implement (mirror `CompanionLink`'s watchdog) → pass → commit `core: bridge link`.

---

### Task 4: sim-glass bridge mode

**Files:** Create `tools/sim-glass/src/main/kotlin/org/openmyglass/simglass/SimBridge.kt`; Modify `Main.kt` (`--bridge` flag), `tools/sim-glass/README.md`; Test `SimBridgeTest.kt`.

**Interfaces — Produces:** `class SimBridge(port: Int = 0, log: (String) -> Unit = {}) : Closeable { fun start(): Int; val navStates: List<Bridge.NavState>; val ended: Int; val icons: Set<String> /* hex sha256 */; fun dropCurrentClient() }` — answers Hello, records NavState/NavEnded/Icon, sends `IconRequest` for any `icon_sha256` it has not received, one client at a time.
- [ ] **Steps:** failing tests `records nav states`, `requests unknown icons once`, `accepts a second session after a drop` → implement → pass → commit `tools: sim-glass bridge mode`.

---

### Task 5: NavigationRelay

**Files:** Create `core/src/main/kotlin/org/openmyglass/core/bridge/NavigationRelay.kt`; Test `NavigationRelayTest.kt` (fake sources as `MutableStateFlow<NavUpdate?>`, fake sender recording frames).

**Interfaces:**
- Consumes: `NavigationSource`, `NavUpdate`, `Bridge.*`.
- Produces: `class NavigationRelay(sources: List<NavigationSource> /* priority order */, icons: (sha256: String) -> ByteArray?, scope: CoroutineScope)` with `fun attach(send: suspend (Bridge.Frame) -> Unit)`, `fun detach()`, `suspend fun onIconRequest(sha256: ByteArray)`.
  - Active source = first in the list whose latest update is non-null (*Review Focus 2*).
  - Sends `NavState` only when it differs from the last sent one (*Review Focus 4*); sends `NavEnded` once when the active source goes `null` and no other source is active.
  - Sends `Icon` for a hash before the first `NavState` that references it, once per attachment; answers `IconRequest`.
  - `attach` after a reconnect resends the current `NavState` and its icon (*Review Focus 3*).
- [ ] **Step 1: Failing tests:** `sends nav state on change only`, `prefers the first active source`, `falls back to the second source when the first ends`, `sends NavEnded once when all sources end`, `sends each icon once before its first reference`, `resends state and icon after reattach`, `answers icon requests`.
- [ ] **Steps 2–4:** RED → implement → GREEN. **Step 5: Commit** `core: navigation relay`.

---

### Task 6: Phone wiring — Maps source, icon encoding, relay to sim

**Files:** Create `MapsNavigationSource.kt`, `IconEncoder.kt` in `android/app/.../nav/`; Modify `OpenMyGlassNotificationListener.kt` (feed Maps and OsmAnd notifications to the sources in addition to capture), `OsmAndNavigationSource.kt` (merge `OsmAndNotificationParser.Extra` into its updates: `instruction` from the notification when present, `eta`), `DebugActivity.kt` (field for bridge host:port, default `10.0.2.2:7824`, connect button, shows link state and last sent NavState); Tests `IconEncoderTest.kt` (JVM-testable part: size/limit logic on a pure `scaleTo(width, height, max): Pair<Int, Int>` helper), `MapsNavigationSourceTest.kt` (feeds `NotificationText` and posted/removed events).
- **Interfaces — Produces:** `class MapsNavigationSource : NavigationSource { fun onPosted(text: NotificationText); fun onRemoved() }` (name `"gmaps"`); `object IconEncoder { fun scaleTo(w: Int, h: Int, max: Int = 128): Pair<Int, Int>; fun encode(bitmap: android.graphics.Bitmap): Pair<String /*sha256 hex*/, ByteArray /*PNG*/>? }` — returns `null` when the PNG would exceed 16 KiB (*Review Focus 5*).
- [ ] **Step 1: Failing tests:** `scale keeps aspect and caps at 128` (`500×250 → 128×64`, `64×64 → 64×64`); Maps source `posted update becomes current`, `removal emits null`.
- [ ] **Steps 2–4:** RED → implement → GREEN (`./gradlew :android:app:testDebugUnitTest`).
- [ ] **Step 5: End-to-end on the emulator:** `./gradlew :tools:sim-glass:run --args="--bridge --port 7824"`; in the app connect to `10.0.2.2:7824`; run an OsmAnd navigation with `tools/emulator/simulate-route.py`, then a Google Maps one. Expected in the sim log: Hello, NavState changes with maneuver/distance/street/ETA, Icon frames (hash only logged), NavEnded at the end; no duplicate consecutive NavStates. Record the observations in `docs/navigation-sources.md` ("Relay").
- [ ] **Step 6: Commit** `android: relay navigation to the bridge`; update `docs/progress.md`.
