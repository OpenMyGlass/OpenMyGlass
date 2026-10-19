# Sources of the analyzed binaries

Binaries live only in `_re/` (excluded from git). This file records where they come from and how they were verified.

## MyGlass (phone side)

| Field | Value |
|---|---|
| Local file | `_re/apk/myglass-3.5.8.apk` |
| Source | https://archive.org/download/myglass-3.5.8-android/myglass-3.5.8.apk (item `myglass-3.5.8-android`, uploaded by a third party on 2026-03-03) |
| Downloaded | 2026-10-19 |
| Package | `com.google.glass.companion` (checked with `aapt`) |
| Version | versionName `3.5.8`, versionCode `5331` |
| SDK | minSdk 15, targetSdk 22, compiled against platform 26 (O) |
| File SHA-1 | `95ec430bc6ed4bc96fa90484fd3704381b8f90bc` (matches the SHA-1 published by archive.org) |
| File SHA-256 | `4f8ae50dbec42b79c67c95a155df9744a3bccc67c46d19d8a5ff2eb60ba2dcd2` |
| Signature | v1 + v2 valid (`apksigner verify`) |
| Certificate DN | `CN=Unknown, OU="Google, Inc", O="Google, Inc", L=Mountain View, ST=CA, C=US` |
| Certificate SHA-1 | `24bb24c05e47e0aefa68a58a766179d9b613a600` |
| Certificate SHA-256 | `3d7a1223019aa39d9ea0e3436ab7c0896bfb4fb679f4de5fe7c23f326c8f994a` |

**Why we trust it:** the APK comes from a third-party upload, but the signature is valid and certificate SHA-1
`24bb24c0…a600` is Google's app-signing key for official apps (the same as YouTube, Google Photos and Google
Translate, according to APKPure download pages). The size (12,674,492 bytes) matches the one APKMirror lists for
3.5.8 (uploaded there on 2017-06-19). APKMirror itself blocks automated downloads (HTTP 403).

**2014 version for comparison:** not retrieved. APKMirror (3.4.0 from 2014-11-05, 3.3.0, 2.2.0) needs a browser.
Optional per the plan; to fetch manually if the 3.5.8 analysis leaves doubts.

## Glass Explorer Edition XE24 system image (Glass side)

| Field | Value |
|---|---|
| Official page | https://developers.google.com/glass/tools-downloads/system |
| Factory image | https://storage.googleapis.com/support-kms-prod/bTh25b2gcZx5f7apQdJU3lULYTTBoZDHqdsr → `_re/xe24/signed-glass_1-img-5585826.zip` |
| Downloaded | 2026-10-19 |
| Size | 442,720,482 bytes |
| SHA-1 | `46430bc827267796566f0a73d9b503059759561c` — **matches** the official checksum |
| SHA-256 | `76a9d3822077e993bba62c586e5e7cbb1217a81171d78033ff7c06ba285f2363` |
| Boot image | https://storage.googleapis.com/glass_gfw/glass_1-img-5585826/boot.img — returns `AccessDenied` (391-byte XML error, not an image). The `boot.img` inside the factory image is used instead. |
| Kernel sources | https://android.googlesource.com/kernel/omap/+/glass-omap-xrr02 |

### Image contents

- `system.img` is an **Android sparse** image: converted to ext4 with `simg2img` and extracted with `debugfs -R "rdump / system"`.
- Factory-image `boot.img`: SHA-1 `d9794236c2fbd1ecb020b9d71b3a85db3c91a960`. It differs from the separately downloadable `boot.img` (`4e582d8d…`), which the official page associates with the rooting procedure.
- **Build:** `Google/glass_1/glass-1:4.4.4/XRH35/5585826:user/release-keys`, i.e. **Android 4.4.4** (API 19), not 4.4.2. Glass apps report versionName `XE22.0`, but the image is the official XE24 one.
- 47 APKs in `system/app`, `system/priv-app` and `system/framework`. Glass APKs ship `classes.dex` inside the APK; some AOSP apps are odex-only.

### APKs of interest for the Glass side

Integrity is guaranteed by the official SHA-1 of the image. Their META-INF declares a v2 signature that is missing, so `apksigner` prints no certificate (see [`emulator-feasibility.md`](emulator-feasibility.md)).

| APK | Package | SHA-256 (prefix) | Role (from analysis) |
|---|---|---|---|
| `priv-app/MyGlassWear.apk` | `com.google.glass.companion` | `b66c9de34432cad4` | Android Wear node on Glass (GmsCore-based) |
| `priv-app/GlassBluetooth.apk` | `com.google.glass.bluetooth` | `b621f843709abff9` | **companion RFCOMM server**, Bluetooth profiles |
| `priv-app/GlassHome.apk` | `com.google.glass.home` | `41d0b6bc418eddf8` | timeline, home, navigation, Wi-Fi setup receiver |
| `priv-app/GlassLiteSetup.apk` | `com.google.glass.litesetup` | `b199acde10a861a6` | account-free first-run setup |
| `priv-app/GlassSync.apk` | `com.google.glass.sync` | `b03f3b763a558633` | sync (timeline/media) |
| `priv-app/GlassPhone.apk` | `com.google.glass.phone` | `89613adb6d0b188e` | calls and SMS through the phone |
| `priv-app/GlassRemoteIme.apk` | `com.google.glass.remoteime` | `15f083280d2eb697` | remote keyboard input from the phone |
| `priv-app/GlassSettings.apk` | `com.google.glass.settings.ui` | `85cc5247b1ee32b5` | settings, Wi-Fi |
| `priv-app/GlassVoice.apk` | `com.google.glass.voice` | `68519445fea45c6d` | dictation and voice commands |
| `priv-app/GlassCamera.apk` | `com.google.glass.camera` | `72ec3959e16cb11f` | photo/video |
| `framework/glass-core.jar` | — | `9715b1e44bf46e75` | shared Glass framework library |
| `framework/glass-services.jar` | — | `28d5011501eaa556` | Glass system services |
