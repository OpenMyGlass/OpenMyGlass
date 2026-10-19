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
| `protocol/` | Message definitions (`.proto` or schema), versioned |
| `core/` | Transport-independent feature logic |
| `android/` | Android companion app |
| `glass-xe/` | GDK bridge app for Explorer XE24 (if the architecture needs one) |
| `tools/` | Simulated peers, test harnesses, adb and reverse-engineering scripts |

## Clean-room approach

No Google binaries, system images or decompiled code are committed.
Analysis happens in a local `_re/` folder (git-ignored); the specification in `docs/` bridges analysis
and implementation, which is original code. `tools/check-clean-room.sh` guards against accidental commits.

## License

[GPL-3.0](LICENSE)
