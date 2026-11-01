# OpenMyGlass

Open-source, cloud-free companion for **Google Glass Explorer Edition (XE24)**.
A clean-room rebuild of what the MyGlass app offered: Google Maps navigation on the glasses, full phone
notifications, Wi-Fi setup, screencast, battery, photo/video sync, Glassware management and shared GPS.
Android first, with an architecture ready for iOS.

> Status: **Phase 2 – reverse engineering**. See [`docs/progress.md`](docs/progress.md).

## Layout

| Folder | Content |
|---|---|
| `docs/` | Reverse-engineering, protocol and architecture documentation |
| `rust/` | **Cross-platform companion core (Rust)** — shared by phone and PC; see [`docs/companion-rust-core.md`](docs/companion-rust-core.md) |
| `protocol/` | Message definitions (`.proto` or schema), versioned |
| `core/` | Transport-independent feature logic (Android/Kotlin) |
| `android/` | Android companion app |
| `glass-xe/` | GDK bridge app for Explorer XE24 (if the architecture needs one) |
| `tools/` | Simulated peers, test harnesses, adb and reverse-engineering scripts |

## Cross-platform core

The companion must run on a **phone and on a PC** (including a PC with no Bluetooth), and
share one codebase with the Rust Glass-side daemon across both target OSes. The
[`rust/`](rust/) workspace is that shared core: a versioned serde wire protocol, a
power-aware transport (Bluetooth RFCOMM default, Wi-Fi/TCP on-demand/fallback), persistent
one-time pairing with mutual authentication, offline Wi-Fi QR bootstrap, and a CLI/daemon
frontend (`omg`) with a built-in simulated Glass. Architecture and the glassd protocol
contract: [`docs/companion-rust-core.md`](docs/companion-rust-core.md).

```sh
cd rust && cargo test && cargo clippy --all-targets && cargo fmt --check
# Try it: terminal 1
cargo run -p omg-cli -- sim-glass --listen 127.0.0.1:7700
# terminal 2
cargo run -p omg-cli -- link --connect 127.0.0.1:7700 --pair --locale it
```

## Clean-room approach

No Google binaries, system images or decompiled code are committed.
Analysis happens in a local `_re/` folder (git-ignored); the specification in `docs/` bridges analysis
and implementation, which is original code. `tools/check-clean-room.sh` guards against accidental commits.

## License

[GPL-3.0](LICENSE)

Third-party components and their licenses: [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
