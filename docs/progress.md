# Progress

Full project brief: [`project-brief.md`](project-brief.md) (English) — original Italian version: [`project-prompt.it.md`](project-prompt.it.md).
Current plan: [`superpowers/plans/2026-10-19-phase-0-2-reverse-engineering.md`](superpowers/plans/2026-10-19-phase-0-2-reverse-engineering.md).

## Phase status

| Phase | Status |
|---|---|
| 0 — Tool inventory and repo setup | ✅ done (2026-10-19); `tools/check-tools.sh` checks tools, `tools/check-clean-room.sh` blocks Google material |
| 1 — Binary retrieval (MyGlass APK, XE24 image) | ✅ done (2026-10-19), see [`sources.md`](sources.md) |
| 2 — Reverse engineering and mapping | ✅ done (2026-10-19): [`feature-inventory.md`](feature-inventory.md), [`protocol/`](protocol/), [`glass-internals.md`](glass-internals.md), [`cloud-dependencies.md`](cloud-dependencies.md), [`prior-art.md`](prior-art.md) — **waiting for the owner's review of the inventory** |
| 3 — Architecture | ✅ approved by the owner (2026-10-19): option **C (hybrid)**, [`architecture.md`](architecture.md) |
| 4 — Android implementation | 🔄 4a done (core, CompanionLink, sim-glass); 4b done: navigation sources on the phone (OsmAnd AIDL source, notification capture), measured in [`navigation-sources.md`](navigation-sources.md); 4c done: notification parsers, bridge protocol `openmyglass.v1`, `BridgeLink`, `NavigationRelay`, phone relay verified end to end against `sim-glass --bridge`; next: Glass bridge app + navigation card (4d) |
| 5 — iOS plan | ⏳ (only on request) |

## Phase 0 — Tool inventory (macOS arm64, 2026-10-19)

| Tool | Status | Notes |
|---|---|---|
| git, gh | ✅ | gh authenticated |
| Java (OpenJDK 21) | ✅ | |
| Python 3, Homebrew | ✅ | |
| Android SDK | ✅ | `~/Library/Android/sdk` (platform-tools, build-tools, emulator, system-images, cmdline-tools); `adb` not on PATH (scripts use the full path) |
| jadx | ✅ 1.5.6 | brew |
| apktool | ✅ 3.0.3 | brew |
| Ghidra | ❌ | optional, native code only |
| gradle, kotlin | ❌ | will come through the Gradle wrapper in Phase 4 |
| protoc | ✅ libprotoc 36.2 | brew `protobuf` |
| e2fsprogs (debugfs), simg2img, 7z | ✅ | brew; used to extract the XE24 image |
| apksigner, aapt | ✅ | build-tools 37.0.0 |
| REA MCP (`rea-agents` 6.7.0) | ✅ | registered at project scope (`claude mcp add --scope local`); uses JADX/Ghidra as providers |
| GDK add-on (API 19) | ❌ | no longer in the official SDK repository; to be sourced for `glass-xe/` in Phase 4 |

## Phase 2 — Decompilation (2026-10-19)

Decompiled with jadx 1.5.6 and apktool 3.0.3 into `_re/` (not versioned). The indexes in `_re/index/` count files or strings, not contents:

| Target | .java | UUIDs | proto classes | RFCOMM | Glass intents | URLs | Wearable API |
|---|---|---|---|---|---|---|---|
| MyGlass 3.5.8 (phone) | 3780 | 9 | 134 | 7 | 155 | 39 | 81 |
| MyGlassWear (Glass) | 2806 | 7 | 62 | 4 | 153 | 63 | 299 |
| GlassBluetooth | 943 | 7 | 29 | 9 | 114 | 15 | 0 |
| GlassHome | 2385 | 7 | 64 | 4 | 197 | 51 | 243 |
| GlassLiteSetup | 577 | 6 | 13 | 4 | 18 | 5 | 0 |
| GlassSync | 2293 | 7 | 62 | 4 | 150 | 51 | 239 |
| GlassPhone | 1976 | 7 | 27 | 4 | 99 | 50 | 176 |
| GlassRemoteIme | 1285 | 7 | 105 | 4 | 70 | 5 | 0 |
| GlassSettings | 2723 | 7 | 65 | 4 | 227 | 57 | 261 |
| GlassVoice | 2737 | 9 | 99 | 4 | 145 | 65 | 285 |
| GlassCamera | 2321 | 7 | 61 | 4 | 169 | 51 | 240 |
| glass-core.jar | 142 | 0 | 0 | 1 | 43 | 0 | 0 |
| glass-services.jar | 83 | 0 | 0 | 0 | 18 | 0 | 0 |

The similar counts across Glass apps (7 UUIDs, 4 RFCOMM files) come from a shared library bundled into each app.

## Key findings so far

- Companion channel: RFCOMM secure, Glass is server, UUID `F15CC914-…`, varint-delimited protobuf `Envelope`s ([`protocol/transport.md`](protocol/transport.md)).
- **No application-level authentication and no Google account needed** on XE24 ([`protocol/handshake.md`](protocol/handshake.md)).
- Glass natively handles timeline cards, location, navigation intents, Wi-Fi setup, screencast, touch injection, device info ([`protocol/messages.md`](protocol/messages.md)).
- The MyGlass backend (`glass.google.com`) is gone (HTTP 404); native turn-by-turn needs `com.google.glass.maps`, which is not in the XE24 image ([`cloud-dependencies.md`](cloud-dependencies.md)).
- Third-party apps on Glass cannot write native timeline cards, capture the screen or inject input; the companion channel can ([`glass-internals.md`](glass-internals.md)).

## Hardware checklist

Things verified only against `sim-glass` so far; to confirm on a real Glass XE24:

- [ ] XE24 accepts our 6.10 handshake (no OTA prompt, no disconnect).
- [ ] Real RFCOMM framing behaves like the TCP tests (partial reads, large frames).
- [ ] `glassInfoResponseG2C` values match the device (battery, storage, name, software version, 640×360).
- [ ] `timezoneC2G` updates the Glass clock.
- [ ] The firmware's ping (SDP query / `PingCompanion`) triggers our reconnection.
- [ ] Navigation updates keep coming while the phone projects to Android Auto **in a real car** (verified on the emulator with the Desktop Head Unit, 2026-10-19: both OsmAnd AIDL and the Maps notification keep working).

## Later phases (agreed with the owner, 2026-10-19)

- **ROM and kernel:** slim down the XE24 ROM and optimize the kernel for battery and performance. The owner asked to compare three strategies: (A) debloat stock XE24, (B) start from a community port and reinstall Google's card UI, (C) fully reverse-engineer Google's UI and hardware controls and reimplement them open source; voice commands to be forwarded to the phone for offline transcription. Research notes are in the local `_re/research/` (not versioned) and will be turned into a design. Kernel and ROM scripts go into **separate repos**, grouped with this one in a **GitHub organization** with a shared Project board (the owner creates the organization).
- This conflicts with the architecture's "no root on Glass" anti-goal: it will be an optional, separate track, decided with the owner when it starts.

## Decisions

- 2026-10-19 — License **GPL-3.0**.
- 2026-10-19 — Public repo; reverse-engineering workspace in `_re/`, excluded from git.
- 2026-10-19 — **No Glass hardware available for now.** Start with emulator/simulator; hardware tests (adb, HCI snoop) stay planned but postponed.
- 2026-10-19 — All repo content in **English**.
- 2026-10-19 — Architecture option **C (hybrid)**. Owner priorities: navigation (car and on foot, with Android Auto), notifications with dictated replies (offline transcription on the phone), photo/video sync on Wi-Fi + charging. Navigation source: an app with a navigation API preferred (OsmAnd candidate), Google Maps notification as fallback.

## Open issues

- Glass XE24 emulation **not feasible** on this Mac (see [`emulator-feasibility.md`](emulator-feasibility.md)). Hardware-free testing through a simulated peer over TCP and an arm64 AVD with GDK stubs.
- GDK add-on missing from the official SDK repository: to be sourced for Phase 4.
- `MyGlassWear.apk` (Glass side) runs an Android Wear node through GmsCore: a second, Google-Play-services-based channel next to the companion RFCOMM channel. Its role and whether it still works is to be assessed in the feature inventory.
- `Envelope.notificationC2G` (field 45) is received by XE24 but no consumer was found in the decompiled APKs; odex-only APKs (e.g. GlassSystemUI) not checked yet.
- Glass Proxy (`fafbdd20-…`) stream format and SMS reply path not analyzed yet.
- `gdk.jar` redistribution status to check before Phase 4.
