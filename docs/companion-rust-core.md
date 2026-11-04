# Companion architecture: the shared Rust core

Status: **bootstrap** (2026-11-01). Implements the backbone of the cross-platform
companion decided in the owner + Claude design session. Sources: the companion spec
(`openmyglass-companion-spec-2026-11-01`), the two-track strategy
(`openmyglass-strategy-two-tracks-2026-11-01`), and the connectivity/pairing decisions
in that strategy note.

This document covers the **new cross-platform companion core** under
[`rust/`](../rust/). It does not replace the reverse-engineering docs in
[`protocol/`](protocol/) or the Android-first implementation described in
[`architecture.md`](architecture.md); it is the next step those pointed to — see
[Relationship to the existing code](#relationship-to-the-existing-code).

## 1. Why a Rust core

Two owner decisions force the architecture:

1. **The companion must run on a phone AND on a PC** — including a PC with **no
   Bluetooth**. A single Android app cannot satisfy that.
2. **The same code should serve Model A and Model B.** The Glass-side daemon/apps
   (`glassd`, glass-apps) are Rust precisely so one codebase runs on the Android ROM
   (Model A) and on plain Linux (Model B). The companion mirrors that: one Rust core,
   thin platform frontends.

So the companion is a **shared Rust core** (transport + protocol + logic) with thin
frontends:

```
          ┌─────────────────────────────────────────────┐
          │                 frontends                    │
          │  omg-cli (now)   Android UI    desktop UI     │
          │   CLI/daemon      (JNI/UniFFI)  (later)       │
          └───────────────┬─────────────────────────────┘
                          │  calls
          ┌───────────────▼─────────────────────────────┐
          │                 omg-core                     │
          │  identity · persistent pairing · auth ·      │
          │  session state machine (backbone)            │
          └───────┬──────────────────────┬──────────────┘
                  │                       │
        ┌─────────▼────────┐    ┌─────────▼─────────┐
        │   omg-transport  │    │    omg-protocol   │
        │ power-aware pick │    │ versioned message │
        │ BT RFCOMM / TCP  │    │ schema (serde)    │
        └──────────────────┘    └───────────────────┘
                                          ▲
                                          │ SAME crate
                                   ┌──────┴───────┐
                                   │   glassd     │  (Glass side, separate repo)
                                   └──────────────┘
```

The core builds on Linux with **no platform-locked dependencies** (tokio, serde,
ed25519-dalek, postcard). Bluetooth — the one inherently platform-specific piece — sits
behind a trait and is stubbed for now (see §4).

### Frontend strategy

- **Now:** `omg-cli` (`omg`), a CLI/daemon that already does the whole backbone. It also
  bundles a **simulated Glass** (`omg sim-glass`) so the link runs end-to-end without
  hardware.
- **Later:** the mobile/desktop UI is a thin shell over `omg-core`. On Android the core
  is exposed via JNI/UniFFI; the UI only renders and provides the BT socket. No protocol
  or session logic is reimplemented per platform.

## 2. Workspace layout (`rust/`)

| Crate | Role |
|---|---|
| `omg-protocol` | Versioned wire message schema (serde). The contract shared with glassd. Also the Wi-Fi QR bootstrap payload type. |
| `omg-transport` | Frame codec + `Transport` trait; `TcpTransport` (works), `BtRfcommTransport` (stub); power-aware `TransportManager`. |
| `omg-core` | ed25519 identity, persistent one-time pairing, mutual auth, notes, and the handshake as a pure state machine. Free of tokio; see below. The async session and the simulated Glass are behind the `async-link` feature. |
| `omg-qr` | Renders a `WifiBootstrap` as a scannable QR (terminal + SVG), offline. |
| `omg-cli` | The `omg` binary: `identity`, `qr`, `sim-glass`, `link`, `daemon`, `note-add`, `notes-sync`. |

Build/test: `cd rust && cargo test --all-features --workspace && cargo clippy --all-targets --all-features && cargo fmt --check`.

### Decision: `omg-core` is free of tokio (2026-11-04)

By default `omg-core` builds without tokio, `futures` or `omg-transport`, and CI checks this. Whatever needs them (`session`, `simglass`, `notes::link`) is behind the `async-link` feature, which only `omg-cli` turns on.

Why:

- **Its other two users cannot use a runtime anyway.** glassd is a single-threaded `poll` loop on a 32-bit Glass and needs the Glass side of notes. A phone app owns its socket (`BluetoothSocket`, Core Bluetooth, `Network.framework`) and must feed bytes in from whichever thread the platform gives it.
- **The logic was already synchronous.** The state machines take events and `now_ms` and return effects. Only the convenience loops around them are async, so the runtime was a dependency of the crate and not of the logic.
- **iOS.** Nothing in the shared API then assumes a particular socket or radio, and no runtime thread pool has to survive iOS background suspension.
- **Size.** glassd's release build is `opt-level = "s"` for a device with little room.

What it costs: the handshake exists twice, as `companion_link` (sync, companion role) and `session` (async, both roles). `tests/companion_link.rs` runs one against the other over TCP. If the CLI ever moves to the sync driver, `session` can go.

A feature was chosen over a separate crate because it moves no code, and `cargo tree` in CI is enough to stop tokio creeping back in.

## 3. Protocol (`omg-protocol`)

One `Frame` enum carries every message; the wire form is `u32` big-endian length +
[postcard](https://docs.rs/postcard) payload. postcard is compact and `no_std`-friendly —
good for the Glass. A JSON projection (serde) is available for debugging. `PROTOCOL_VERSION`
(currently `1`) is exchanged in `Hello`.

**Both ends depend on this one crate**, so the schema cannot drift. glassd implements the
same `Frame`. New variants are appended at the end of the enum (postcard encodes enums by
discriminant).

See [the protocol contract for glassd](#6-what-glassd-must-implement).

## 4. Transport (`omg-transport`) — power-aware, BT-default, Wi-Fi on-demand

The Glass keeps **Wi-Fi off by default** (a top battery drain); the companion is the
gateway and the normal link is **Bluetooth**, which is on anyway for A2DP/HFP. Wi-Fi is
brought up **on demand** for bandwidth (media sync, OTA) or as the **fallback** for a PC
without BT.

`TransportManager` picks the **lowest marginal-power** transport available right now and
falls back (`rank()` is a pure, unit-tested function):

1. **Reuse a radio already on** (≈ zero marginal cost): Wi-Fi up → TCP; BT up → RFCOMM.
2. **Low-bandwidth, nothing up:** prefer Bluetooth (BLE → RFCOMM); cheaper than staying
   Wi-Fi-associated.
3. **High-bandwidth, or no BT:** Wi-Fi/TCP wins.

Either side can request *"bring Wi-Fi up for a big transfer, then drop it"* via the
`WifiBringUpRequest` / `WifiBringUpResult` / `WifiTearDown` frames.

**What works today:** the **TCP path** is fully implemented (dial + listen). The **BT
RFCOMM path is a stub behind the `Transport` trait** — BT hardware integration is
platform-specific (Linux BlueZ, Android `BluetoothSocket`). It reports `available() =
false`, so the manager falls back to TCP. A BlueZ implementation lands behind the
`linux-bt` feature; glassd is the RFCOMM server advertising the OpenMyGlass service UUID
`6f6d672d-636f-6d70-616e-696f6e2d7631` (clean-room, **not** Google's companion UUID).

## 5. Identity, pairing, auth (`omg-core`)

**Persistent, one-time pairing to a single companion.** Each side has a long-term
**ed25519 identity** (the trust anchor), persisted with the bonded peer in a JSON state
file (atomic temp-write + rename). On Model B this file must live on persistent storage
(`/var/lib/glassd`) or the Glass forgets its companion on reboot.

Handshake (implemented end-to-end, over both an in-memory pipe and TCP, with tests):

1. **Hello** both ways: role, app version, device name, identity public key, capabilities.
   Protocol-version check.
2. **Authorize / pair:**
   - already bonded + same key → accept (persistent re-use);
   - already bonded + **different** key → **reject** (`Error{UnknownPeer}`, close) — this
     is what "ignore others until unpaired" means;
   - not bonded + pairing permitted → one-time pairing: `PairRequest` → `PairAccept` →
     `PairResult`, both sides store each other's key.
3. **Mutual auth:** each side sends an `AuthChallenge` nonce; the peer returns an
   `AuthResponse` = ed25519 signature over `"openmyglass-companion-auth-v1" || nonce`;
   each verifies against the **bonded** public key. Both must pass. This authenticates
   every reconnection (including over TCP, where there is no BT bonding to lean on).

Auto-reconnect over the cheapest transport, and mDNS discovery of the companion (its IP
changes), are the next steps; the frames and the bootstrap address hint are already in
place.

## 6. Wi-Fi QR bootstrap (`omg-qr` + `omg-protocol::bootstrap`)

Breaks the chicken-and-egg (the link needs connectivity; configuring Wi-Fi needs the
link) **offline**: the companion shows a QR, the Glass camera scans it, glassd joins
Wi-Fi and finds the companion. No Google servers.

Payload (`WifiBootstrap`): `ssid`, `auth` (Open/WEP/WPA-PSK/WPA3-SAE), `psk`, `hidden`,
optional `companion_addr` (`ip:port` hint), `mdns_service`, and a one-time `pair_token`
binding the first pairing to the companion that made the QR.

Two encodings:
- `OMG1:<base64url(postcard(WifiBootstrap))>` — the OpenMyGlass format, **what glassd
  decodes**. Carries the extras a plain Wi-Fi QR cannot.
- `WIFI:S:…;T:…;P:…;H:…;;` — the standard string generic cameras understand (join only).

glassd needs a QR **decoder** (e.g. `rqrr` / `quircs`) to turn a camera frame into the
string, then calls `WifiBootstrap::decode`. `omg-qr` only encodes.

## 7. i18n

Scope (owner, 2026-11-01): **Italian, English, French, Spanish** only — all Latin, no
RTL, one small always-resident font. The protocol carries a `Locale` enum; the companion
pushes it with `SetLocale` and the Glass acks/notifies with `LocaleChanged`. UI string
tables live in glass-apps (shared by Model A and B); the companion is just the picker.

## 8. What is implemented vs scaffolded

**Implemented end-to-end, with tests** (`cargo test`, 25 tests):
- Framing + protocol round-trips; power-selection ranking matrix.
- Identity, persistent store, one-time pairing, mutual auth, unknown-peer rejection,
  bond re-use on reconnect.
- Session backbone: Hello, heartbeat (Ping/Pong + RTT), `SetLocale`/`LocaleChanged`,
  `StatusRequest`/`Status`, `BatteryStatus` push, notification forward + action echo.
- TCP transport (dial + listen); QR bootstrap generation + payload round-trip.
- `omg` CLI driving all of the above against `omg sim-glass`.
- Notes with attachments and deferred sync, including the speech engine traits: see
  [`companion-notes-sync.md`](companion-notes-sync.md). glassd does not speak it yet.

**Scaffolded** — typed in `omg-protocol`, handlers stubbed (`// SCAFFOLD`): radio control
+ airplane, on-demand Wi-Fi coordination, time sync, location + turn-by-turn nav,
dictation relay, media sync (Wi-Fi + BT fallback), live captions / translation, assistant
push-to-talk, add-on management, settings mirror, screencast, remote find/lock/wipe, OTA,
tethering. Plus: real BT RFCOMM transport, mDNS discovery, auto-reconnect.

## 9. What glassd must implement to match

glassd (Glass side) depends on the **`omg-protocol` crate** and must:

1. **Framing:** `u32` big-endian length + postcard payload per `Frame`; cap
   `MAX_FRAME_LEN` (16 MiB).
2. **RFCOMM server:** advertise service UUID `6f6d672d-636f-6d70-616e-696f6e2d7631` over
   SDP and accept one companion connection (also accept TCP on the Wi-Fi/PC path).
3. **Handshake as the Glass role:** send/receive `Hello`; run `authorize` (accept a
   one-time pairing when unbonded; **reject** any peer that is not the bonded one with
   `Error{UnknownPeer}`); run the mutual `AuthChallenge`/`AuthResponse`/`AuthResult`
   signing over `"openmyglass-companion-auth-v1" || nonce`.
4. **Persist** its ed25519 identity and the bonded companion across reboots
   (`/var/lib/glassd`).
5. **Backbone handlers:** answer `Ping` with `Pong`; answer `StatusRequest` with
   `Status`; push `BatteryStatus`; apply `SetLocale` and reply `LocaleChanged`; render
   `NotificationPosted` as a timeline card and send `NotificationAction` on user input;
   honour `NotificationRemoved`.
6. **QR bootstrap decode:** scan the camera, decode an `OMG1:` string via
   `WifiBootstrap::decode`, join Wi-Fi (rfkill + wpa_supplicant + udhcpc), then connect to
   `companion_addr` or discover via mDNS.

`omg-core`'s `simglass` module (`crates/omg-core/src/simglass.rs`) is an **executable
reference** for items 3 and 5: each `match` arm is exactly what glassd must do.

## Relationship to the existing code

The repo already contains an Android-first companion (Kotlin `core/`, `android/`,
`glass-xe/`, and the reconstructed `protocol/*.proto`) built in Phases 3–6. That work
stands; this Rust core is the cross-platform evolution the owner asked for on 2026-11-01,
when two new constraints appeared: **PC support (no BT)** and **one codebase across Model
A and Model B**.

- The Rust `omg-protocol` is the forward contract for the **glassd-based** architecture
  (Rust on the Glass). It intentionally mirrors the proven message set of the existing
  `protocol/openmyglass/v1/bridge.proto` (Nav, Notification, Dictation, Media, Location)
  so the migration carries over known-good shapes, while adding the backbone the new
  design needs (identity/pairing/auth, locale, radio/airplane, on-demand Wi-Fi, status).
- The Kotlin `core/` remains the current Android implementation; as the Rust core gains
  features, the Android app becomes a thin frontend over it (JNI/UniFFI) rather than a
  parallel implementation.
- No reverse-engineered or Google material is used here; this is original code.
