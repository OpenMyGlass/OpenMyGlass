# Prior art

Cloned shallowly into `_re/prior-art/` on 2026-10-19. Licenses are taken from each repo's `LICENSE` file or README.
OpenMyGlass is GPL-3.0: Apache-2.0 and MIT code can be reused with attribution (Apache-2.0 is GPL-3.0-compatible); code with no license cannot be reused.

| Project | License | Last commit | Covers | Matches our spec? | Reusable for us |
|---|---|---|---|---|---|
| [thorikawa/GlassBluetoothProtocol](https://github.com/thorikawa/GlassBluetoothProtocol) | Apache-2.0 (README) | 2013-12-21 | Android library for the **original companion protocol**: RFCOMM to `F15CC914-…`, `.proto` for Envelope, Timeline, screenshot, GlassInfo, MotionEvent | ✅ Same UUID; its `Envelope` fields (1–5, 9, 11, 13–17, 20–22) match ours number for number. It predates fields 23–47. | Confirms option A works from a third-party app. Our schema is newer and complete; their code is a small reference only. |
| [thorikawa/GlassRemote](https://github.com/thorikawa/GlassRemote) | Apache-2.0 | 2013-12-21 | Phone app on top of GlassBluetoothProtocol: screencast and remote control of the Glass UI | ✅ uses the same protocol | Proof that screencast + `motionC2G` touch injection work through the companion channel. |
| [thalmiclabs/myo-glass-companion](https://github.com/thalmiclabs/myo-glass-companion) | Apache-2.0 | 2020-09-10 (archived product) | Drives the native Glass UI with Myo gestures through the companion protocol (UUID `F15CC914-…`) | ✅ same UUID and approach | More evidence for option A (input injection). Myo-specific parts are irrelevant. |
| [inckie/AnotherGlass](https://github.com/inckie/AnotherGlass) | MIT | 2026-10-19 (active) | **Own protocol + bridge app** (option B): phone Host Service, XE and EE Glass apps; GPS forwarding, notifications (ongoing and one-shot), media control, Wi-Fi credentials (XE), tilt-to-wake (EE); Bluetooth (own UUID `05f2934c-…`) or Wi-Fi/TCP transport, QR code for IP | n/a — different protocol, does not use the companion service | **High:** working GDK toolchain (`compileSdk 37`, `minSdk/targetSdk 19`, `gdk.jar` as `compileOnly` stub in `glass-xe/libs`, old Java target for Glass); Wi-Fi/TCP and Bluetooth transports; the notification and GPS forwarding design. |
| IrisGlass ([code.nym.sh/kennethnym/aris-old](https://code.nym.sh/kennethnym/aris-old), `IrisGlass/`) | **none found** (no LICENSE, no README) | 2026-01-10 | XE24 app acting as **BLE central** towards an iPhone peripheral (`IrisCompanion`), GDK LiveCard UI; custom GATT service `A0B0C0D0-E0F0-4A0B-9C0D-0E0F1A2B3C4D` with characteristics `…D1`–`…D3` | n/a | **Ideas only** (no license = all rights reserved): confirms XE24 can run a BLE central to talk to iOS, useful for Phase 5. Its build looks up `gdk.jar` under `add-ons/addon-google_gdk-google-19/libs/`. |

## What still works / what is missing

- **Original protocol (option A):** three independent projects (2013–2015) talked to the Glass companion service from third-party Android apps with no Google account, which matches our handshake analysis. None of them covers XE24-era fields (23–47), location forwarding, Wi-Fi setup over the companion channel, or timeline-item injection for notifications. Nobody has published a full MyGlass replacement on this channel.
- **Bridge-app approach (option B):** AnotherGlass is alive and covers GPS, notifications, Wi-Fi and media control, but it needs its own app on Glass and gives no access to native timeline cards, screencast or input injection (see [`glass-internals.md`](glass-internals.md)).
- **Missing everywhere:** Google Maps turn-by-turn on Glass from the phone's Maps notification, notification replies through `RemoteInput` with dictation, photo/video sync without the cloud, and an iOS path (only an unlicensed BLE prototype).

## GDK toolchain note

The GDK add-on is no longer in the SDK repository (see [`emulator-feasibility.md`](emulator-feasibility.md)). AnotherGlass ships a `gdk.jar` compile stub in its repo, and IrisGlass expects it under `add-ons/addon-google_gdk-google-19/`. For OpenMyGlass the redistribution status of `gdk.jar` (a Google artifact) must be checked before committing it. Alternatives are original stubs, or fetching it at build time from a user-provided path.
