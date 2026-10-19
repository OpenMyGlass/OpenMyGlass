# Phase 4e: Bluetooth bridge and phone foreground service — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Phone and Glass talk over **Bluetooth RFCOMM** (bonding authenticates the link), the phone keeps both links (companion and bridge) alive from a **foreground service** with automatic reconnection, and the Glass bridge app starts at boot.

**Architecture:**
- **Phone:**
  - `RfcommTransport` implements `core`'s `Transport`.
  - A generic `LinkManager` in `core` generalizes the reconnection loop of `ConnectionManager` (connect → handshake → run → backoff) and is reused for the bridge.
  - `OpenMyGlassService`, a foreground service of type `connectedDevice`, owns `NavigationHub` and both managers: the bridge link to our Glass app, and the original companion link to the firmware.
  - A simple settings screen chooses the bonded Glass and the transport (Bluetooth, or TCP for debug).
- **Glass:**
  - `BridgeServer` accepts from both TCP and RFCOMM through a small `ServerChannel` abstraction.
  - A boot receiver starts the bridge/LiveCard service.

The emulator has no real Bluetooth, so RFCOMM is verified with unit tests behind interfaces plus TCP end to end; real RFCOMM goes on the hardware checklist.

**Tech Stack:** Kotlin (phone, core), Java (Glass, API 19), Android Bluetooth Classic API (`createRfcommSocketToServiceRecord`, `listenUsingRfcommWithServiceRecord`), JUnit 5/4.

**Spec:** [`docs/architecture.md`](../../architecture.md) §3, §4 (Transport, state machine, reconnect triggers), §5.1, §6, §8, §10; [`docs/protocol/transport.md`](../../protocol/transport.md); [`docs/glass-app.md`](../../glass-app.md).

## Global Constraints

- **Bridge RFCOMM service:** UUID `8a0dcb95-c455-453f-87b5-d69ddc82b743`, SDP name `OpenMyGlass Bridge`, **secure** (bonded only). Companion RFCOMM: `F15CC914-E4BC-45CE-9930-CB7695385850` (firmware).
- **Phone:** minSdk 29, targetSdk 36. Runtime permissions: `BLUETOOTH_CONNECT` (API 31+, required for `connectedDevice` foreground services), `POST_NOTIFICATIONS` (33+); foreground service type `connectedDevice`.
- **Battery (§6):** no polling; backoff 1 s → 60 s; reconnect triggers on bond/ACL events and the firmware's ping socket (`PingCompanion`, `DC935393-…`).
- **Glass:** API 19, Java; the TCP port stays as a documented debug path.
- English only; no Claude attribution; run `bash tools/check-clean-room.sh` **after** `git add`.

## Review Focus

1. **Bluetooth off, or Glass out of range:** the service backs off and keeps retrying with no busy loop; the notification shows "Glass not reachable" → test in Task 2.
2. **Permission `BLUETOOTH_CONNECT` denied or revoked:** no crash; the service stops with a clear notification, and the UI asks again → test in Task 4.
3. **Glass bridge app not installed** (companion link works, bridge does not): the companion link keeps running, the bridge retries with the maximum backoff → test in Task 2.
4. **App killed by the system or phone rebooted:** the service is restarted by the system; navigation sources rebind → covered by `START_STICKY`, checked manually in Task 5.
5. **Two transports configured** (TCP debug and Bluetooth): exactly one is active for the bridge → test in Task 3.

---

## File Structure

| File | Responsibility |
|---|---|
| `core/.../connection/LinkManager.kt` | generic connect/handshake/run/backoff loop with `state`, `reconnectNow`, `stop` |
| `core/.../connection/ConnectionManager.kt` | becomes a thin `LinkManager` user for the companion link (API unchanged) |
| `core/.../bridge/BridgeManager.kt` | `LinkManager` for the bridge, attaches/detaches `NavigationRelay` sessions |
| `android/app/.../transport/RfcommTransport.kt` | `Transport` over a bonded `BluetoothDevice` and a UUID |
| `android/app/.../service/OpenMyGlassService.kt` | foreground service: owns hub, bridge manager, companion manager, notification |
| `android/app/.../service/Settings.kt` | chosen Glass device address, transport, enable flags (SharedPreferences) |
| `android/app/.../SettingsActivity.kt` | pick bonded Glass, transport, start/stop service, permission requests |
| `glass-xe/app/src/main/java/.../bridge/ServerChannel.java` | accept abstraction: TCP and RFCOMM implementations |
| `glass-xe/app/src/glass/java/.../RfcommServerChannel.java` | `listenUsingRfcommWithServiceRecord` |
| `glass-xe/app/src/glass/java/.../BootReceiver.java` | starts `NavLiveCardService` on boot |

---

### Task 1: `LinkManager` (generic reconnection loop)

**Files:** Create `LinkManager.kt`; Modify `ConnectionManager.kt` to delegate; Tests `LinkManagerTest.kt` (fake transport and session).

**Interfaces — Produces:**
```kotlin
interface LinkSession<H> { suspend fun handshake(): H; suspend fun run(); fun close() }
class LinkManager<H>(transport: () -> Transport?, open: (Channel) -> LinkSession<H>, scope: CoroutineScope,
                     backoff: Backoff = Backoff(), onStateChange: (LinkState<H>) -> Unit = {})
sealed interface LinkState<out H> { Disconnected, Connecting, Handshaking, Connected(hello: H), WaitingToRetry(attempt, delay), Unavailable(reason: String), Stopped }
```
`transport()` returning `null` means "not configured or not available" (e.g. Bluetooth off), giving `Unavailable` plus backoff. The existing `ConnectionManagerTest` must stay green unchanged.
- [ ] Failing tests: `retries with backoff`, `unavailable transport backs off without connecting` (*Review Focus 1*), `reconnectNow skips the wait`, `stop is final`, `a failing session ends and retries`. RED → implement → GREEN; run `ConnectionManagerTest` → still green. Commit `core: generic link manager`.

### Task 2: `BridgeManager`

**Files:** Create `BridgeManager.kt`; Test `BridgeManagerTest.kt` (with `SimBridge` from `:tools:sim-glass` as a test dependency of `core`? No: use an in-test loopback `FakeGlass`-style bridge peer).

**Interfaces — Produces:** `class BridgeManager(transport: () -> Transport?, relay: NavigationRelay, appVersion: String, scope: CoroutineScope, backoff: Backoff = Backoff())` with `state: StateFlow<LinkState<Bridge.Hello>>`, `start()`, `stop()`, `reconnectNow()`. On `Connected` it attaches a relay session, answers `IconRequest`s, and detaches when the session ends.
- [ ] Failing tests: `connects and relays navigation`, `retries when the bridge app is missing` (connection refused → `WaitingToRetry`, capped backoff; *Review Focus 3*), `reattaches after a drop`. RED → GREEN → commit `core: bridge manager`.

### Task 3: Phone `RfcommTransport`, settings and transport selection

**Files:** Create `RfcommTransport.kt`, `Settings.kt`, `TransportFactory.kt` (`fun bridgeTransport(settings, adapter): Transport?` → RFCOMM to the chosen bonded device with the bridge UUID, or TCP for debug, or `null`); Tests `TransportFactoryTest.kt` (pure: settings → which transport kind/target; *Review Focus 5*: exactly one).
- [ ] RED → GREEN → commit `android: RFCOMM transport and settings`.

### Task 4: `OpenMyGlassService` (foreground, `connectedDevice`) and settings UI

**Files:** Create `OpenMyGlassService.kt`, `SettingsActivity.kt`; Modify `AndroidManifest.xml` (service with `foregroundServiceType="connectedDevice"`, permissions `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_CONNECTED_DEVICE`, `BLUETOOTH_CONNECT`, `POST_NOTIFICATIONS`), `NavigationHub.kt` (owned by the service; the debug TCP connect goes through `BridgeManager`), `DebugActivity.kt`; Test `ServiceStatusTest.kt` for the pure `fun notificationText(bridge: LinkState<*>, companion: LinkState<*>, permissionMissing: Boolean): String` (*Review Focus 2*: permission missing → "Bluetooth permission needed").
- The service starts both managers when configured; it reconnects on `BluetoothDevice.ACTION_ACL_CONNECTED` and `BluetoothAdapter.ACTION_STATE_CHANGED` (on) via `reconnectNow()`; it returns `START_STICKY`.
- [ ] RED → GREEN; build; commit `android: foreground service with automatic reconnection`.

### Task 5: Glass RFCOMM server and boot start

**Files:** Create `ServerChannel.java` (+ `TcpServerChannel`), `RfcommServerChannel.java` (glass flavor), `BootReceiver.java`; Modify `BridgeServer.java` (accepts from a list of `ServerChannel`s; one active session across all of them), `NavLiveCardService.java` (TCP + RFCOMM), glass manifest (`BLUETOOTH`, `RECEIVE_BOOT_COMPLETED`, receiver); Test `BridgeServerTest` extended: `sessions from two channels replace each other` (two TCP channels as stand-ins).
- [ ] RED → GREEN; `assembleGlassDebug` (real jar and stubs) → commit `glass-xe: RFCOMM server and boot start`.

### Task 6: End to end and docs

- [ ] **Emulator** (TCP path through the service): settings → transport TCP `127.0.0.1:7825`; start the service; kill and restart the Glass emulator app → the phone reconnects by itself (no Connect button); OsmAnd navigation reaches the card.
- [ ] **Docs:** `docs/glass-app.md` (RFCOMM, boot start, pairing steps), `docs/progress.md` (status; hardware checklist: RFCOMM bridge connects to a bonded XE24, companion link and bridge link at the same time, reconnection after walking out of range, boot start), `android/README.md`.
- [ ] Commit `docs: Bluetooth bridge and foreground service`.
