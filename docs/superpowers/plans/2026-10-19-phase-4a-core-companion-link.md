# Phase 4a: core, CompanionLink and simulated Glass — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A pure-JVM `core` library that connects to Glass over a pluggable transport, completes the original companion handshake, reads device info, sends the time zone and reconnects by itself, proven end-to-end against a simulated Glass peer over TCP.

**Architecture:** Gradle multi-module Kotlin build at the repo root: `:protocol` (protobuf-lite classes from a hand-picked subset of the reconstructed companion schema), `:core` (framing, `Transport`, `CompanionLink`, `ConnectionManager`), `:tools:sim-glass` (a JVM program that behaves like the XE24 companion server). No Android code yet; the RFCOMM transport arrives with the Android app in a later plan.

**Tech Stack:** Kotlin/JVM (JVM toolchain 17), Gradle wrapper with Kotlin DSL and a version catalog, `com.google.protobuf` Gradle plugin with `protobuf-javalite`, kotlinx-coroutines (+ `kotlinx-coroutines-test`), JUnit 5. Use the latest stable versions at implementation time and pin them in `gradle/libs.versions.toml`.

**Spec:** [`docs/architecture.md`](../../architecture.md) §3, §4 (Transport abstraction), §5.1, §7 steps 1–2, §11; protocol facts from [`docs/protocol/transport.md`](../../protocol/transport.md) and [`docs/protocol/handshake.md`](../../protocol/handshake.md).

## Global Constraints

- `core` is a **pure JVM library: no Android imports** (architecture §4).
- Framing on every link: **varint32 length prefix + protobuf message** (transport.md "Framing").
- Companion RFCOMM UUID `F15CC914-E4BC-45CE-9930-CB7695385850`; ping UUID `DC935393-831A-4736-89E3-C8CE5BB20EEA`, SDP name `PingCompanion`.
- `version = (minor << 16) + major`; Glass XE24 is major 6, minor 10 = `655366`. **We send 6.10** (equal major avoids the firmware's OTA prompt; my call, recorded in Task 3).
- **Glass speaks first** in the handshake; the companion answers with one `Envelope`.
- No polling of Glass (battery rule, architecture §6): device info only on connect and on explicit request.
- All repo content in English; Kotlin packages under `org.openmyglass`; never commit Google binaries/code (`bash tools/check-clean-room.sh` before every commit).
- Hardware-free: everything is verified against `sim-glass`; device-only checks go to the hardware checklist in `docs/progress.md`.

## Review Focus

1. **Glass connects but never sends its handshake** (silent peer): the link must time out (10 s) and the manager must back off and retry, not hang → test in Task 5.
2. **A frame arrives split across many reads** (Bluetooth/TCP deliver bytes in pieces): the framing must reassemble it → test in Task 2.
3. **Corrupt or huge length prefix** (line noise, wrong peer): fail with `FramingException`, close, reconnect; never allocate gigabytes → tests in Task 2 and Task 5.
4. **Envelopes carrying fields we don't model** (newer MyGlass/firmware fields such as 45–47): parse fine and ignore → test in Task 1.
5. **A second client connects while one is active** (the firmware accepts one connection at a time): the simulator closes it, and our manager treats that as a failed attempt and backs off → test in Task 6.

---

## File Structure

| File | Responsibility |
|---|---|
| `settings.gradle.kts`, `build.gradle.kts`, `gradle/libs.versions.toml`, `gradlew*`, `gradle/wrapper/*` | root build, modules `:protocol`, `:core`, `:tools:sim-glass` |
| `protocol/build.gradle.kts` | protobuf-lite codegen from `protocol/companion/` |
| `protocol/companion/companion.proto` | the companion subset we use (Task 1) |
| `core/src/main/kotlin/org/openmyglass/core/framing/DelimitedFraming.kt` | varint-delimited read/write |
| `core/src/main/kotlin/org/openmyglass/core/transport/Transport.kt` | `Transport`, `Channel`, `TransportKind` |
| `core/src/main/kotlin/org/openmyglass/core/transport/TcpTransport.kt` | TCP implementation |
| `core/src/main/kotlin/org/openmyglass/core/companion/CompanionConstants.kt` | UUIDs, versions |
| `core/src/main/kotlin/org/openmyglass/core/companion/CompanionLink.kt` | handshake, send/receive envelopes |
| `core/src/main/kotlin/org/openmyglass/core/companion/GlassInfo.kt` | domain models `GlassHello`, `GlassInfo`, `PhoneState` |
| `core/src/main/kotlin/org/openmyglass/core/connection/ConnectionManager.kt` | state machine + reconnection |
| `core/src/main/kotlin/org/openmyglass/core/connection/Backoff.kt` | exponential backoff |
| `tools/sim-glass/src/main/kotlin/org/openmyglass/simglass/SimGlass.kt` | simulated XE24 companion server |
| `tools/sim-glass/src/main/kotlin/org/openmyglass/simglass/Main.kt` | CLI entry point |
| `tools/sim-glass/README.md` | how to run it |

---

### Task 1: Gradle build and companion protocol subset

**Files:**
- Create: root Gradle files and wrapper, `protocol/build.gradle.kts`, `protocol/companion/companion.proto`, `protocol/src/test/kotlin/org/openmyglass/protocol/CompanionProtoTest.kt`
- Modify: `protocol/README.md` (what is in the subset, link to `docs/protocol/`), `.gitignore` (`.gradle/` and `build/` are already there; add `.kotlin/`)

**Interfaces:**
- Produces: Java lite classes in package `org.openmyglass.protocol.companion`, outer class `Companion` (`option java_outer_classname = "Companion"`), proto package `openmyglass.companion`: `Envelope`, `AndroidDeviceInfo`, `CompanionDeviceInfo`, `CompanionFeatureInfo`, `GlassFeatureInfo`, `GlassInfoRequest`, `GlassInfoResponse`.

- [ ] **Step 1:** `brew install gradle`, then `gradle wrapper` at the repo root; create `settings.gradle.kts` including `:protocol`, `:core`, `:tools:sim-glass` (empty modules are fine for now).
- [ ] **Step 2: Write `companion.proto`** (proto2). Copy field names, numbers and types exactly from `docs/protocol/proto/glass-xe24/Proto.proto`, only for:
  - `Envelope`: 1 `version` uint32, 2 `serialNumber` uint32, 3 `timeMillis` uint64, 4 `uptimeMillis` uint64, 9 `timezoneC2G` string, 21 `glassInfoRequestC2G`, 22 `glassInfoResponseG2C`, 30 `androidDeviceInfo`, 31 `companionDeviceInfoC2G`, 32 `companionFeatureInfoC2G`, 33 `glassFeatureInfoG2C`;
  - `AndroidDeviceInfo` (1 `sdkVersion` int32), `CompanionDeviceInfo` (1–3 bools), `CompanionFeatureInfo` (1–2 bools), `GlassFeatureInfo` (1 bool), `GlassInfoRequest` (1–6 bools), `GlassInfoResponse` (1 int32, 2–3 int64, 4–5 string, 6 bool, 7–8 int32).
  Header comment: source schema path and "subset; add fields from the reconstructed schema as features need them".
- [ ] **Step 3: Write the failing tests** in `CompanionProtoTest`:
  - `envelope round-trips version and glass info` — build `Envelope{version=655366, timeMillis=1, glassInfoResponseG2C{batteryLevel=80, deviceName="Glass"}}`, serialize, parse, assert equal values.
  - `unknown fields are ignored` — bytes = a valid `Envelope{version=655366}` followed by field 45 (length-delimited, tag `0x6A 0x01`, wire type 2) carrying 3 arbitrary bytes, plus field 47 (tag `0x7A 0x02`) with 1 byte; parse succeeds and `version == 655366`.
- [ ] **Step 4:** Run `./gradlew :protocol:test` — Expected: FAIL (no codegen yet). Configure `protocol/build.gradle.kts` (protobuf plugin, `lite` option, proto source dir `companion`), rerun — Expected: PASS.
- [ ] **Step 5: Commit** — `bash tools/check-clean-room.sh && git add -A && git commit -m "build: gradle multi-module and companion protocol subset"`.

---

### Task 2: Varint-delimited framing

**Files:**
- Create: `core/build.gradle.kts`, `core/src/main/kotlin/org/openmyglass/core/framing/DelimitedFraming.kt`, `core/src/test/kotlin/org/openmyglass/core/framing/DelimitedFramingTest.kt`

**Interfaces:**
- Produces:
  - `object DelimitedFraming { const val MAX_FRAME_BYTES: Int = 4 * 1024 * 1024; fun writeFrame(out: OutputStream, payload: ByteArray); fun readFrame(input: InputStream, maxBytes: Int = MAX_FRAME_BYTES): ByteArray? }`
  - `class FramingException(message: String) : IOException(message)`
  - `readFrame` returns `null` only on a clean end of stream **before the first byte** of a frame.

- [ ] **Step 1: Write the failing tests:**
  - `writes length as varint then payload` — payload of 3 bytes → output `[0x03, b0, b1, b2]`; payload of 300 bytes → output starts `[0xAC, 0x02]` then 300 bytes.
  - `reads back what it wrote` — two frames written back to back are read in order.
  - `returns null on clean end of stream` — empty input → `null`.
  - `reassembles a frame delivered one byte per read` — wrap the bytes of a 300-byte frame in an `InputStream` whose `read(buf, off, len)` returns at most 1 byte; result equals the payload. *(Review Focus 2)*
  - `throws on truncated payload` — prefix says 10, stream has 4 → `FramingException`.
  - `throws on oversized length` — prefix encodes `MAX_FRAME_BYTES + 1` → `FramingException` without reading further. *(Review Focus 3)*
  - `throws on malformed varint` — six bytes `0xFF` → `FramingException`.
- [ ] **Step 2:** Run `./gradlew :core:test --tests '*DelimitedFramingTest*'` — Expected: FAIL (unresolved reference).
- [ ] **Step 3:** Implement in `DelimitedFraming.kt`. A varint32 is at most 5 bytes; a 6th continuation byte is malformed. Truncation inside the varint or payload is `FramingException`, not `null`.
- [ ] **Step 4:** Rerun — Expected: PASS (7 tests).
- [ ] **Step 5: Commit** — `git commit -m "core: varint-delimited framing"`.

---

### Task 3: Transport abstraction, TCP transport and companion constants

**Files:**
- Create: `core/src/main/kotlin/org/openmyglass/core/transport/Transport.kt`, `TcpTransport.kt`, `core/src/main/kotlin/org/openmyglass/core/companion/CompanionConstants.kt`; tests `TcpTransportTest.kt`, `CompanionConstantsTest.kt`

**Interfaces:**
- Produces:
  - `enum class TransportKind { RFCOMM, TCP, BLE }`
  - `interface Channel : Closeable { val input: InputStream; val output: OutputStream }`
  - `interface Transport { val kind: TransportKind; @Throws(IOException::class) fun connect(): Channel }` — blocking; callers run it on `Dispatchers.IO`.
  - `class TcpTransport(val host: String, val port: Int, val connectTimeoutMs: Int = 5_000) : Transport`
  - `object CompanionConstants { val SECURE_UUID: UUID; const val SOCKET_NAME = "Companion"; val PING_SOCKET_UUID: UUID; const val PING_SOCKET_NAME = "PingCompanion"; const val MAJOR_VERSION = 6; const val MINOR_VERSION = 10; fun calculateVersion(major: Int, minor: Int): Int; fun majorOf(version: Int): Int; fun minorOf(version: Int): Int; val VERSION: Int }`

- [ ] **Step 1: Write the failing tests:**
  - `version matches XE24` — `calculateVersion(6, 10) == 655366`, `calculateVersion(6, 11) == 720902`, `majorOf(655366) == 6`, `minorOf(655366) == 10`, `VERSION == 655366`.
  - `uuids match the spec` — `SECURE_UUID.toString().uppercase() == "F15CC914-E4BC-45CE-9930-CB7695385850"`, `PING_SOCKET_UUID…== "DC935393-831A-4736-89E3-C8CE5BB20EEA"`.
  - `tcp transport exchanges bytes` — a local `ServerSocket(0)` echoes; `TcpTransport("127.0.0.1", port).connect()` writes 3 bytes and reads them back; `kind == TCP`.
  - `tcp transport fails fast when nothing listens` — connecting to a closed port throws `IOException` within `connectTimeoutMs`.
- [ ] **Step 2:** Run `./gradlew :core:test` — Expected: FAIL.
- [ ] **Step 3:** Implement. Record in the KDoc of `VERSION`: "We present 6.10, the XE24 value: an equal major avoids the firmware's OTA prompt (handshake.md)."
- [ ] **Step 4:** Rerun — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "core: transport abstraction, TCP transport, companion constants"`.

---

### Task 4: CompanionLink — handshake, device info, time zone

**Files:**
- Create: `core/src/main/kotlin/org/openmyglass/core/companion/GlassInfo.kt`, `CompanionLink.kt`; test `CompanionLinkTest.kt` with an in-memory channel pair (`PipedInputStream`/`PipedOutputStream`) and a scripted fake Glass in the test.

**Interfaces:**
- Consumes: `Channel`, `DelimitedFraming`, `CompanionConstants`, `Companion.*` protos.
- Produces:
  - `data class PhoneState(val sdkVersion: Int, val timeZoneId: String, val wifiConnected: Boolean, val interactive: Boolean, val gpsEnabled: Boolean, val photoSyncEnabled: Boolean = false, val keyboardTextEntrySupported: Boolean = false)`
  - `data class GlassHello(val remoteVersion: Int, val glassSdkVersion: Int?, val wallpaperFeatureEnabled: Boolean)`
  - `data class GlassInfo(val batteryLevel: Int?, val storageTotalBytes: Long?, val storageAvailableBytes: Long?, val deviceName: String?, val softwareVersion: String?, val needSetup: Boolean?, val screenWidthPx: Int?, val screenHeightPx: Int?)` — fields absent on the wire are `null`.
  - `sealed interface CompanionEvent { data class GlassInfoReceived(val info: GlassInfo) : CompanionEvent; data class Other(val envelope: Companion.Envelope) : CompanionEvent }`
  - `class CompanionLink(channel: Channel, phone: () -> PhoneState, clock: () -> Long = System::currentTimeMillis, uptime: () -> Long = { System.nanoTime() / 1_000_000 })` with:
    - `suspend fun handshake(timeout: Duration = 10.seconds): GlassHello` — reads Glass's first envelope, then sends ours with `version = CompanionConstants.VERSION`, `timeMillis`, `uptimeMillis`, `androidDeviceInfo`, `companionDeviceInfoC2G`, `companionFeatureInfoC2G`, `timezoneC2G`. On timeout, EOF or framing error: closes the channel and throws `IOException`.
    - `suspend fun requestGlassInfo()` — sends `glassInfoRequestC2G` with all six flags `true`.
    - `suspend fun sendTimeZone(id: String)`
    - `fun events(): Flow<CompanionEvent>` — cold flow reading envelopes until EOF (completes) or error (throws).
    - `fun close()`
    - Writes are serialized (one writer at a time); every outgoing envelope carries `version`, `timeMillis`, `uptimeMillis`.

- [ ] **Step 1: Write the failing tests:**
  - `handshake waits for glass then answers` — fake Glass sends `Envelope{version=655366, androidDeviceInfo{sdkVersion=19}, glassFeatureInfoG2C{isWallpaperFeatureEnabled=false}}`; `handshake()` returns `GlassHello(655366, 19, false)`; the envelope fake Glass reads has `version == 655366`, `androidDeviceInfo.sdkVersion == 34` (from `PhoneState`), `timezoneC2G == "Europe/Rome"`, `companionDeviceInfoC2G.gpsEnabled == true`.
  - `handshake does not send before glass speaks` — fake Glass stays silent for 200 ms; nothing is readable on its side during that time.
  - `handshake times out on a silent glass` — `handshake(timeout = 500.milliseconds)` throws `IOException`; the channel is closed. *(Review Focus 1)*
  - `glass info request and response` — after handshake, `requestGlassInfo()` → fake Glass sees all six request flags `true`, replies `glassInfoResponseG2C{batteryLevel=80, deviceName="Glass", glassScreenWidthPixels=640, glassScreenHeightPixels=360}`; `events()` emits `GlassInfoReceived(GlassInfo(batteryLevel=80, deviceName="Glass", screenWidthPx=640, screenHeightPx=360, others null))`.
  - `time zone is sent` — `sendTimeZone("America/New_York")` → fake Glass reads `timezoneC2G == "America/New_York"`.
  - `events complete on clean disconnect` — fake Glass closes its output → the flow completes without error.
- [ ] **Step 2:** Run `./gradlew :core:test --tests '*CompanionLinkTest*'` — Expected: FAIL.
- [ ] **Step 3:** Implement. Blocking reads run on `Dispatchers.IO`; `withTimeout` + closing the channel unblocks a pending read.
- [ ] **Step 4:** Rerun — Expected: PASS (6 tests).
- [ ] **Step 5: Commit** — `git commit -m "core: companion link handshake, device info and time zone"`.

---

### Task 5: ConnectionManager — state machine and reconnection

**Files:**
- Create: `core/src/main/kotlin/org/openmyglass/core/connection/Backoff.kt`, `ConnectionManager.kt`; tests `BackoffTest.kt`, `ConnectionManagerTest.kt` (fake `Transport` handing out scripted in-memory channels; `kotlinx-coroutines-test` virtual time).

**Interfaces:**
- Consumes: `Transport`, `CompanionLink`, `GlassHello`, `CompanionEvent`, `FramingException`.
- Produces:
  - `class Backoff(val initial: Duration = 1.seconds, val max: Duration = 60.seconds, val factor: Double = 2.0) { fun next(): Duration; fun reset() }` — 1 s, 2 s, 4 s … capped at 60 s.
  - `sealed interface ConnectionState` with `data object Disconnected`, `data object Connecting`, `data object Handshaking`, `data class Connected(val hello: GlassHello)`, `data class WaitingToRetry(val attempt: Int, val delay: Duration)`, `data object Stopped`, all implementing it.
  - `class ConnectionManager(transport: Transport, phone: () -> PhoneState, scope: CoroutineScope, backoff: Backoff = Backoff(), handshakeTimeout: Duration = 10.seconds, linkFactory: (Channel) -> CompanionLink = …)` with `val state: StateFlow<ConnectionState>`, `val events: SharedFlow<CompanionEvent>`, `fun start()`, `fun stop()`, `fun reconnectNow()` (skips the current wait; to be called on Bluetooth bond/ACL events and on the `PingCompanion` socket later), `suspend fun requestGlassInfo()`, `suspend fun sendTimeZone(id: String)` — both throw `IllegalStateException` when the state is not `Connected`.
  - On `Connected`: sends `requestGlassInfo()` once (architecture §5.1: on connect, no polling) and resets the backoff.

- [ ] **Step 1: Write the failing tests:**
  - `backoff doubles and caps` — `next()` × 8 → 1, 2, 4, 8, 16, 32, 60, 60 s; after `reset()` → 1 s.
  - `connects, handshakes and requests glass info` — states observed in order: `Disconnected → Connecting → Handshaking → Connected`; the fake Glass receives a `glassInfoRequestC2G` after the handshake.
  - `retries with backoff when connect fails` — transport throws `IOException` twice then succeeds; states include `WaitingToRetry(1, 1s)` and `WaitingToRetry(2, 2s)` before `Connected`; virtual time advanced accordingly.
  - `silent glass leads to retry, not hang` — first channel never sends a handshake; after `handshakeTimeout` the state goes to `WaitingToRetry`. *(Review Focus 1)*
  - `corrupt frame leads to retry` — first channel sends `0xFF ×6`; state reaches `WaitingToRetry`, second attempt connects. *(Review Focus 3)*
  - `disconnect after connected triggers reconnect and backoff reset` — connected, then the fake closes the stream → `WaitingToRetry(1, 1s)` (reset because the previous attempt had connected).
  - `reconnectNow skips the wait` — while `WaitingToRetry(_, 4s)`, `reconnectNow()` → `Connecting` without advancing 4 s.
  - `stop closes the link and stays stopped` — `stop()` → `Stopped`; no further `connect()` calls on the transport.
- [ ] **Step 2:** Run `./gradlew :core:test --tests '*Backoff*' --tests '*ConnectionManager*'` — Expected: FAIL.
- [ ] **Step 3:** Implement as one coroutine loop owned by `scope`; the link's `events()` are forwarded to `events`.
- [ ] **Step 4:** Rerun — Expected: PASS (8 tests). Then `./gradlew :core:test` — all `core` tests PASS.
- [ ] **Step 5: Commit** — `git commit -m "core: connection manager with reconnection"`.

---

### Task 6: sim-glass and end-to-end test

**Files:**
- Create: `tools/sim-glass/build.gradle.kts` (Kotlin `application`, main class `org.openmyglass.simglass.MainKt`), `SimGlass.kt`, `Main.kt`, `tools/sim-glass/src/test/kotlin/org/openmyglass/simglass/EndToEndTest.kt`, `tools/sim-glass/README.md`
- Modify: `tools/README.md` (add sim-glass row), `docs/progress.md` (Phase 4 status; add a **Hardware checklist** section with: "XE24 accepts our 6.10 handshake", "real RFCOMM framing matches TCP behavior", "firmware ping triggers our reconnection")

**Interfaces:**
- Consumes: `:protocol`, `DelimitedFraming`, `CompanionConstants`; the test also uses `TcpTransport`, `ConnectionManager`.
- Produces:
  - `data class SimGlassConfig(val port: Int = 0, val glassSdkVersion: Int = 19, val version: Int = CompanionConstants.VERSION, val info: GlassInfo = GlassInfo(80, 16_000_000_000, 8_000_000_000, "Sim Glass", "XE24", false, 640, 360), val silent: Boolean = false)`
  - `class SimGlass(config: SimGlassConfig) : Closeable { fun start(): Int /* bound port */; val receivedTimeZones: List<String>; val clientVersions: List<Int>; fun dropCurrentClient(); override fun close() }`
  - Behavior mirroring XE24 (handshake.md, messages.md): sends its handshake first (`version`, `androidDeviceInfo{sdkVersion}`, `glassFeatureInfoG2C{false}`); reads the client's; logs a warning when the client's major > 6; answers `glassInfoRequestC2G` with `info`; records `timezoneC2G`; **one client at a time** — a second connection is accepted and closed immediately; `silent = true` never sends the handshake.
  - CLI: `./gradlew :tools:sim-glass:run --args="--port 7823 [--silent] [--battery 55]"` prints one line per received envelope.

- [ ] **Step 1: Write the failing end-to-end tests** (`EndToEndTest`, real TCP on `127.0.0.1`, real time with short durations via `Backoff(initial = 100.milliseconds, max = 400.milliseconds)` and `handshakeTimeout = 1.seconds`):
  - `connects and receives glass info` — manager reaches `Connected(GlassHello(655366, 19, false))`; `events` emits `GlassInfoReceived` with `batteryLevel == 80`, `deviceName == "Sim Glass"`.
  - `time zone reaches glass` — after connect, `sendTimeZone("Europe/Rome")` → `sim.receivedTimeZones` contains `"Europe/Rome"` (the handshake one counts too).
  - `reconnects after glass drops the link` — `sim.dropCurrentClient()` → state passes through `WaitingToRetry` and returns to `Connected` within 2 s.
  - `second client is refused` — a raw second TCP connection to the sim reads end-of-stream immediately while the manager stays `Connected`. *(Review Focus 5)*
  - `silent glass is retried` — sim with `silent = true` → manager cycles `Handshaking → WaitingToRetry` and never reaches `Connected` within 2.5 s.
- [ ] **Step 2:** Run `./gradlew :tools:sim-glass:test` — Expected: FAIL.
- [ ] **Step 3:** Implement `SimGlass` (accept loop on a background thread; per-connection handler) and `Main.kt` argument parsing (`--port`, `--silent`, `--battery`).
- [ ] **Step 4:** Rerun — Expected: PASS (5 tests). Then the full suite: `./gradlew test` and `for t in tools/tests/test-*.sh; do bash $t; done` — all PASS.
- [ ] **Step 5: Manual check:** `./gradlew :tools:sim-glass:run --args="--port 7823"` starts and prints its listening line; stop it with Ctrl-C.
- [ ] **Step 6: Commit** — `bash tools/check-clean-room.sh && git add -A && git commit -m "tools: sim-glass and end-to-end companion test"`.
