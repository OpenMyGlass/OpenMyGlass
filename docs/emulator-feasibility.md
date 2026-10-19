# Spike: emulating Glass XE24 without hardware

**Question:** can we run the Glass side of XE24 (the whole system or its companion apps) in an emulator on this Mac (Apple Silicon, arm64), to test the companion without Glass?

**Short answer:** **no.** Neither the full XE24 system nor the Glass system apps run here in a useful way. For development we will use a **simulated peer** and, if the architecture includes a bridge app, an **arm64 AVD with a 640×360 display and GDK stubs**. Until hardware is available, original firmware behavior can only be checked by reading the decompiled code.

Date: 2026-10-19. Environment: macOS arm64, Android Emulator 36.5.11, cmdline-tools 9862592. All probe material is in `_re/emulator/` and is throwaway.

## What was tried

### A. XE24 image or stock API 19 ARM AVD

- XE24 is **32-bit ARMv7** (OMAP4, `board=glass_1`; libraries in `system/lib` are 32-bit ARM EABI5 ELF) with OMAP HALs, not goldfish.
- Before transplanting, I checked whether the official `system-images;android-19;default;armeabi-v7a` runs. Boot fails immediately:
  `FATAL | CPU Architecture 'arm' is not supported by the QEMU2 emulator, (the classic engine is deprecated!)`.
- The SDK ships a `qemu-system-armel` binary, but the emulator refuses ARM32 images under QEMU2. The classic engine, the only one that ran them, has been removed.
- **Outcome:** blocked at the root. If the stock API 19 system does not boot, a transplanted XE24 will not either.

### B. Glass system apps on a modern AVD (API 28 arm64-v8a)

- The APKs in `system/priv-app` have a META-INF that declares a missing v2 signature (`INSTALL_PARSE_FAILED_NO_CERTIFICATES`). They install only after re-signing with a debug key.
- `GlassBluetooth` uses `sharedUserId=android.uid.system`, so it cannot be installed without the platform key (`INSTALL_FAILED_SHARED_USER_INCOMPATIBLE`).
- `MyGlassWear` and `GlassHome` ship native libraries for `lib/armeabi` only, while the AVD exposes only `arm64-v8a`: `INSTALL_FAILED_NO_MATCHING_ABIS`. Apple Silicon cannot run 32-bit ARM code, so no arm64 AVD on this Mac offers that ABI.
- `MyGlassWear` without native libraries installs, but crashes as soon as any component starts: `NoClassDefFoundError: com/google/android/glass/hidden/HiddenSystemProperties`, because the Glass framework is missing (`glass-core.jar`, `glass-services.jar`, `com.google.android.glass.*` classes).
- **Outcome:** the Glass framework and the ARM32 native code are hard dependencies; rebuilding them in an emulator would cost more than the hardware.

### C. GDK add-on

- The GDK package (`addon-google_gdk-google-19`) is **no longer in the official SDK repository** (`addon2-1.xml`). It must be sourced elsewhere (archives, mirrors) to build a possible bridge app; to be handled in Phase 4.

## Side finding

The components registered by `MyGlassWear.apk` (XE24, package `com.google.glass.companion`) belong to the **Android Wear / clockwork framework**, e.g. `com.google.android.clockwork.stream.bridger.NotificationBridgerService`, `com.google.android.clockwork.host.DispatchingWearableListenerService` and `com.google.android.wearable.app.RemoteActionReceiver`. Glass runs a Wear node through GmsCore, alongside the companion RFCOMM channel (see [`protocol/`](protocol/)).

## Recommendation for hardware-free testing

1. **Simulated Glass peer** in `tools/` (Python or JVM) speaking our message model over **TCP**. The emulated phone reaches it via `adb reverse`/`adb forward`; TCP replaces RFCOMM, which the emulator does not provide (no AVD has real Bluetooth Classic).
2. **Bridge app on an arm64 AVD** with a 640×360 display profile and **GDK class stubs** (`CardBuilder`, `LiveCard`, …) for UI and logic, if Phase 3 picks option B or C. Bridge-app code must avoid ARM32 native libraries.
3. **Original-protocol emulation (option A):** only verifiable with hardware or, partially, by replaying in the simulated peer the sequences derived from the decompiled code. This does not test the real firmware.
4. **Untested alternative:** an x86_64 Linux host with an old emulator that still has the classic engine might boot API 19 ARM. Transplanting XE24 would still hit the OMAP HAL problems. Not recommended unless strongly needed.
