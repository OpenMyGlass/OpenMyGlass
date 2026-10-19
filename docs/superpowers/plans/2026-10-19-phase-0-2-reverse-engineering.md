# Phases 0–2: tools, binary retrieval, reverse engineering — Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reach the complete MyGlass feature inventory (`docs/feature-inventory.md`) with the protocol specification, Glass internal APIs, cloud dependencies and prior art documented, without Glass hardware.

**Architecture:** Analysis work, not product work. Binaries and decompiled code live only in `_re/` (git-ignored); the repo receives documentation, reconstructed `.proto` files and small helper scripts in `tools/`. Without glasses, verification happens on the decompiled code and, if feasible, on an emulator running the XE24 image.

**Tech Stack:** REA MCP (`rea-agents`, registered locally; use its tools for assisted analysis on top of jadx), jadx, apktool, apksigner (Android build-tools), protobuf (`protoc`), e2fsprogs/simg2img for the system image, Android Emulator, bash, Python 3.

**Spec:** [`docs/project-brief.md`](../../project-brief.md) (Step 0, Phases 1–2) + the "no hardware" constraint in [`docs/progress.md`](../../progress.md).

## Global Constraints

- Target: Glass Explorer Edition XE-C, firmware **XE24** (Android 4.4.2 / API 19, GDK).
- Expected MyGlass package: `com.google.glass.companion` — **verify**, do not assume.
- MyGlass version: **the latest available (2017 or later)**; optionally a 2014 one for comparison.
- XE24 image: only from the official Google for Developers "System and Kernel Downloads" page.
- **Never commit** APKs, system images, `.dex`, `.jar`, decompiled code. Everything in `_re/`.
- Documentation, not copied code: describe classes/messages, do not paste their bodies.
- Do not invent UUIDs, protobuf fields, intents or formats: every statement cites the class/file in `_re/` it comes from, or is marked **[unverified]**.
- No Glass hardware: no on-device tests in these phases.
- Update `docs/progress.md` at the end of each task; commit often.
- **Mandatory stop** after Task 10: show the inventory to the user, do not start Phase 3.
- (Added 2026-10-19 at the user's request) All repo content in **English**.

## Review Focus

1. **Accidental commit of Google material** (APKs, dex, decompiled sources pasted into `docs/`): the clean-room check is expected to block it before push → test in Task 1.
2. **Mirror APK with a non-Google or repackaged signature**: expected to be discarded, not analyzed → certificate check in Task 2.
3. **APK distributed as split/bundle (`.apks`/`.xapk`)**: expected to be recognized and rebuilt, or rejected with a clear message → handled in the Task 2 script.
4. **System image in sparse format or with nested partitions**: expected a documented conversion, not a silently empty extraction → check of the number of extracted APKs in Task 3.
5. **Obfuscated (ProGuard) names that differ between versions**: every class reference in the docs must name the APK version it comes from → citation rule in Tasks 6–10.

---

## File Structure

| File | Responsibility |
|---|---|
| `tools/check-tools.sh` | Checks the required tools and their versions |
| `tools/check-clean-room.sh` | Fails if git-tracked files contain Google binaries or code |
| `tools/re/apk-info.sh` | Prints package, version, SHA-256 of file and certificate of an APK |
| `docs/sources.md` | Provenance, versions, hashes and signatures of all analyzed binaries |
| `docs/emulator-feasibility.md` | Result of the XE24 emulation spike and hardware-free test strategy |
| `docs/protocol/README.md` | Protocol specification index |
| `docs/protocol/transport.md` | Transport, UUID/SDP, pairing, framing, keep-alive, reconnection |
| `docs/protocol/handshake.md` | Handshake, authentication, account/token requirements, XE24 behavior |
| `docs/protocol/messages.md` | Message catalog and per-feature sequences |
| `docs/protocol/proto/*.proto` | Schemas reconstructed from the generated classes |
| `docs/glass-internals.md` | Glass-side intents, broadcasts, providers, services and their reachability |
| `docs/cloud-dependencies.md` | What went through Google servers and how to replace it |
| `docs/prior-art.md` | Analysis of existing projects and their licenses |
| `docs/feature-inventory.md` | Feature inventory (the stop-gate deliverable) |

---

### Task 1: Tools and clean-room guard

**Files:**
- Create: `tools/check-tools.sh`, `tools/check-clean-room.sh`
- Modify: `docs/progress.md` (Phase 0 table)

**Interfaces:**
- Produces: `tools/check-tools.sh` (exit 0 if everything is present), `tools/check-clean-room.sh` (exit 1 on violations; used by later tasks before every commit).

- [ ] **Step 1: Install the tools**

```bash
brew install jadx apktool protobuf e2fsprogs simg2img p7zip
export ANDROID_HOME="$HOME/Library/Android/sdk"
ls "$ANDROID_HOME/build-tools"   # note the latest version
```

If `simg2img` is not available in brew, record it in `docs/progress.md` and use `brew install android-platform-tools` + an alternative documented in Task 3.

- [ ] **Step 2: Write `tools/check-tools.sh`**

```bash
#!/usr/bin/env bash
# Checks the tools required by phases 0–2.
set -u
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
BT="$(ls -d "$SDK"/build-tools/* 2>/dev/null | sort -V | tail -1)"
missing=0
check() { if command -v "$1" >/dev/null 2>&1 || [ -x "$2" ]; then printf 'ok    %s\n' "$1"; else printf 'MISS  %s\n' "$1"; missing=1; fi; }
check jadx ""
check apktool ""
check protoc ""
check debugfs "$(brew --prefix e2fsprogs 2>/dev/null)/sbin/debugfs"
check simg2img ""
check 7z ""
check adb "$SDK/platform-tools/adb"
check apksigner "$BT/apksigner"
check emulator "$SDK/emulator/emulator"
exit $missing
```

- [ ] **Step 3: Run it**

Run: `bash tools/check-tools.sh`
Expected: all lines `ok`, exit 0. Every `MISS` must be fixed or documented before continuing.

- [ ] **Step 4: Write the clean-room guard test (must fail)**

```bash
mkdir -p /tmp/cr-test && cd /tmp/cr-test && git init -q
printf 'PK' > fake.apk && printf 'package com.google.glass.companion;\n' > Leak.java
git add -f fake.apk Leak.java
bash /Users/giorgio/Documents/google-glass-project/tools/check-clean-room.sh; echo "exit=$?"
```

Expected (before implementation): `No such file` → the test fails.

- [ ] **Step 5: Write `tools/check-clean-room.sh`**

```bash
#!/usr/bin/env bash
# Fails if tracked (or staged) files contain Google binaries or Google source code.
set -u
fail=0
files="$(git ls-files --cached)"
bad_ext="$(printf '%s\n' "$files" | grep -Ei '\.(apk|apks|xapk|dex|odex|img|jar)$' | grep -v 'gradle-wrapper.jar' || true)"
if [ -n "$bad_ext" ]; then echo "Tracked binaries:"; echo "$bad_ext"; fail=1; fi
leaks="$(printf '%s\n' "$files" | grep -E '\.(java|smali|kt)$' | xargs -I{} grep -lE '^(package|\.class .*L)com/?\.?google' {} 2>/dev/null || true)"
if [ -n "$leaks" ]; then echo "Sources with a Google package:"; echo "$leaks"; fail=1; fi
if printf '%s\n' "$files" | grep -q '^_re/'; then echo "_re/ is tracked"; fail=1; fi
[ $fail -eq 0 ] && echo "clean-room ok"
exit $fail
```

- [ ] **Step 6: Run the test again**

Run: the same command as Step 4.
Expected: lists `fake.apk` and `Leak.java`, `exit=1`. Then, in the repo: `bash tools/check-clean-room.sh` → `clean-room ok`, exit 0.

- [ ] **Step 7: Update `docs/progress.md` and commit**

Update the Phase 0 table with the real versions (`jadx --version`, `apktool --version`, `protoc --version`).

```bash
chmod +x tools/*.sh
bash tools/check-clean-room.sh && git add tools docs/progress.md && git commit -m "tools: tool check and clean-room guard"
```

---

### Task 2: Retrieve and verify the MyGlass APK

**Files:**
- Create: `tools/re/apk-info.sh`, `docs/sources.md`
- Local only: `_re/apk/`

**Interfaces:**
- Consumes: `apksigner` from Task 1.
- Produces: `_re/apk/myglass-<versionName>.apk` (latest), optionally `_re/apk/myglass-2014-<versionName>.apk`; `tools/re/apk-info.sh <apk>`.

- [ ] **Step 1: Write `tools/re/apk-info.sh`**

```bash
#!/usr/bin/env bash
# Usage: apk-info.sh <file.apk> — prints package, version, file hash and signing certificate.
set -eu
apk="$1"
case "$apk" in *.apks|*.xapk) echo "ERROR: split bundle ($apk). Extract base.apk with 7z and run again." >&2; exit 2;; esac
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
BT="$(ls -d "$SDK"/build-tools/* | sort -V | tail -1)"
"$BT/aapt" dump badging "$apk" | grep -E "^package:"
echo "file_sha256: $(shasum -a 256 "$apk" | cut -d' ' -f1)"
"$BT/apksigner" verify --print-certs "$apk" | grep -E 'Signer #1 certificate (DN|SHA-256)'
```

- [ ] **Step 2: Try it on known inputs**

Run: `touch /tmp/x.xapk && bash tools/re/apk-info.sh /tmp/x.xapk; echo "exit=$?"`
Expected: "split bundle" message, `exit=2`.
Run: `bash tools/re/apk-info.sh /nonexistent.apk; echo "exit=$?"`
Expected: `aapt` error on a missing file, exit ≠ 0.

- [ ] **Step 3: Download the APK**

Look for MyGlass on APKMirror (preferred source: it verifies the original signature) and, as an alternative, archive.org. Save it in `_re/apk/`. Pick the latest version (≥2017); if available, also a 2014 one.

- [ ] **Step 4: Verify**

Run: `bash tools/re/apk-info.sh _re/apk/<file>.apk`
Expected: `package: name='...'` (record the real value), certificate with a Google DN (`O=Google Inc.` or similar). **If the DN is not Google, discard the APK** and record it.

- [ ] **Step 5: Write `docs/sources.md`**

For each binary: name, exact source URL, download date, package, versionName/versionCode, file SHA-256, certificate DN and SHA-256, notes.

- [ ] **Step 6: Commit**

```bash
bash tools/check-clean-room.sh && git add tools/re docs/sources.md && git commit -m "docs: MyGlass APK provenance and verification"
```

---

### Task 3: Retrieve and extract the XE24 image

**Files:**
- Modify: `docs/sources.md`
- Local only: `_re/xe24/`

**Interfaces:**
- Produces: `_re/xe24/system/` (extracted filesystem), `_re/xe24/apk-list.txt` (system APK list with paths).

- [ ] **Step 1: Download the image**

From the Google for Developers "System and Kernel Downloads" page (Glass Explorer Edition), download the **XE24** system image into `_re/xe24/`. Record URL, file name, SHA-256 (`shasum -a 256`) and, if the page publishes them, compare with the official hashes.

- [ ] **Step 2: Extract the archive and identify `system.img`**

```bash
cd _re/xe24 && 7z x <archive> && file *.img
```

Expected: `system.img` described as "Android sparse image" or "Linux ext4 filesystem".

- [ ] **Step 3: Convert and extract**

```bash
simg2img system.img system.raw.img   # only if sparse
mkdir -p system && "$(brew --prefix e2fsprogs)/sbin/debugfs" -R "rdump / system" system.raw.img
find system -name '*.apk' > apk-list.txt && wc -l apk-list.txt
```

Expected: dozens of APKs (0 means extraction failed: stop and diagnose).

- [ ] **Step 4: Identify the companion side**

```bash
grep -iE 'companion|bluetooth|setup|home|nav|notif|screencast|sync|glass' apk-list.txt
```

Record the candidate APKs in `docs/sources.md` with package (via `apk-info.sh`) and why they were picked.

- [ ] **Step 5: Commit**

```bash
bash tools/check-clean-room.sh && git add docs/sources.md && git commit -m "docs: XE24 system image provenance and companion-side APKs"
```

---

### Task 4: Spike — XE24 emulator and hardware-free test strategy

**Files:**
- Create: `docs/emulator-feasibility.md`
- Modify: `docs/progress.md`

**Interfaces:**
- Consumes: `_re/xe24/` from Task 3.
- Produces: a recommendation on how to test the Glass side in Phase 4 (read by Task 10 and Phase 3).

Timebox: **2 hours**. The spike is an investigation: what it produces is throwaway and stays in `_re/emulator/`.

- [ ] **Step 1: Look for existing work**

Search for projects that booted Glass XE images in an emulator (e.g. "Glass emulator XE system image", "GDK emulator"). Record links and status, without assuming they work.

- [ ] **Step 2: Try A — XE24 image on an API 19 AVD**

Check whether the emulator on Apple Silicon can run an ARMv7 API 19 image (`sdkmanager --list | grep 'android-19'`). If so, try booting an API 19 AVD with `system.img` replaced by the XE24 one. Record the steps and the result (boot yes/no, `emulator -verbose -show-kernel` logs).

- [ ] **Step 3: Try B — Glass system APKs on a stock API 19 AVD**

If Try A fails: install the Glass-side companion APKs (Task 3) on a stock API 19 AVD with the GDK add-on, if available. Record which ones start and which fail because of system dependencies.

- [ ] **Step 4: Bluetooth in the emulator**

Check and document that the emulator does not expose real Bluetooth Classic; record the test alternative (TCP transport via `adb forward`/`adb reverse` to a simulated peer in `tools/`).

- [ ] **Step 5: Write `docs/emulator-feasibility.md`**

Sections: question, what was tried, result of each try, recommendation (e.g. "Python simulated peer + API 19 AVD for the bridge app" or "XE24 in the emulator works for X but not Y").

- [ ] **Step 6: Commit**

```bash
bash tools/check-clean-room.sh && git add docs/emulator-feasibility.md docs/progress.md && git commit -m "docs: emulator feasibility spike for XE24"
```

---

### Task 5: Decompilation and class index

**Files:**
- Local only: `_re/jadx/<name>/`, `_re/apktool/<name>/`, `_re/index/<name>.txt`

**Interfaces:**
- Produces: decompiled sources for the MyGlass APK and for every Glass-side companion APK; indexes of key strings used by Tasks 6–10.

- [ ] **Step 1: Decompile**

```bash
for apk in _re/apk/*.apk <Glass-side APKs from Task 3>; do n=$(basename "$apk" .apk)
  jadx -d "_re/jadx/$n" --show-bad-code "$apk"
  apktool d -f -o "_re/apktool/$n" "$apk"
done
```

Expected: every `_re/jadx/<n>/sources` folder is non-empty.

- [ ] **Step 1b: Assisted analysis with REA** — point the `rea` MCP tools at the APKs in `_re/apk/` and on the Glass side for a first map of components, Bluetooth entry points and protobuf classes; use it to steer the indexes below, verifying every result against the jadx code. If REA does not support an APK, record it and continue with jadx.

- [ ] **Step 2: Build the indexes**

```bash
mkdir -p _re/index
for d in _re/jadx/*; do n=$(basename "$d")
  grep -rhoE '[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}' "$d" | sort -u > "_re/index/$n.uuids.txt"
  grep -rlE 'extends (GeneratedMessage|GeneratedMessageLite|MessageNano)|MessageNano' "$d" > "_re/index/$n.protos.txt"
  grep -rlE 'listenUsingRfcomm|createRfcommSocket|BluetoothServerSocket' "$d" > "_re/index/$n.rfcomm.txt"
  grep -rhoE '"(com\.google\.glass|com\.google\.android\.glass)[A-Za-z0-9_.]*"' "$d" | sort -u > "_re/index/$n.intents.txt"
  grep -rhoE 'https?://[A-Za-z0-9./_-]+' "$d" | sort -u > "_re/index/$n.urls.txt"
done
wc -l _re/index/*
```

- [ ] **Step 3: Record in `docs/progress.md`** the summary of the counts (not the contents) and commit only `docs/progress.md`.

---

### Task 6: Protocol specification

**Files:**
- Create: `docs/protocol/transport.md`, `docs/protocol/handshake.md`, `docs/protocol/messages.md`, `docs/protocol/proto/*.proto`
- Modify: `docs/protocol/README.md`

**Interfaces:**
- Consumes: `_re/index/*` indexes from Task 5.
- Produces: compilable `.proto` files, used by Phase 3 to choose between options A/B/C.

Citation rule: every statement names `APK@version → class (obfuscated name and, if recoverable, role)`.

- [ ] **Step 1: Transport** — starting from `*.rfcomm.txt` and `*.uuids.txt`, document in `transport.md`: socket type, SDP UUID and service name (is Glass server or client?), pairing, framing (length/delimiters, endianness), keep-alive, timeouts, reconnection.
- [ ] **Step 2: Handshake** — in `handshake.md`: initial sequence, exchanged fields, presence of Google tokens/accounts, signature or identity checks on the companion, XE24 behavior after the update that removed the account requirement (cite the code that proves it).
- [ ] **Step 3: Rebuild the `.proto` files** — from the classes in `*.protos.txt` recover messages, field numbers, types and enums into `docs/protocol/proto/`. Doubtful fields get a `// [unverified]` comment.
- [ ] **Step 4: Check they compile**

Run: `protoc -I docs/protocol/proto --descriptor_set_out=/dev/null docs/protocol/proto/*.proto`
Expected: no output, exit 0.

- [ ] **Step 5: Messages and sequences** — in `messages.md`, for each feature the message sequence (text diagram) and the error handling.
- [ ] **Step 6: Commit**

```bash
bash tools/check-clean-room.sh && git add docs/protocol && git commit -m "docs: companion protocol specification and reconstructed protos"
```

---

### Task 7: Glass-side internal APIs

**Files:**
- Create: `docs/glass-internals.md`

- [ ] **Step 1:** From the `AndroidManifest.xml` files in `_re/apktool/<Glass APK>/`, list exported activities, services, receivers and providers, with the required permissions (`android:permission`, `protectionLevel`).
- [ ] **Step 2:** Cross-check with `*.intents.txt` to find intents/broadcasts the companion used for navigation, timeline, notifications, location, screencast.
- [ ] **Step 3:** For each, one table row: name, purpose, sender, permission, **reachable by third-party apps? (yes / signature|system / root)**, evidence (file + manifest line).
- [ ] **Step 4:** Commit: `bash tools/check-clean-room.sh && git add docs/glass-internals.md && git commit -m "docs: Glass-side internal APIs"`.

---

### Task 8: Cloud dependencies

**Files:**
- Create: `docs/cloud-dependencies.md`

- [ ] **Step 1:** From `*.urls.txt` and the classes using them, classify every endpoint: function (auth, timeline/Mirror, Glassware install, nav routing, contacts, media sync, telemetry), side (phone/glasses).
- [ ] **Step 2:** For each: what happens today if the server does not answer (from the code), and the proposed local replacement, or "not replaceable" with the reason.
- [ ] **Step 3:** Commit: `bash tools/check-clean-room.sh && git add docs/cloud-dependencies.md && git commit -m "docs: Google cloud dependencies and local replacements"`.

---

### Task 9: Prior art

**Files:**
- Create: `docs/prior-art.md`
- Local only: `_re/prior-art/`

- [ ] **Step 1: Clone**

```bash
mkdir -p _re/prior-art && cd _re/prior-art
for r in thorikawa/GlassBluetoothProtocol thorikawa/GlassRemote thalmiclabs/myo-glass-companion inckie/AnotherGlass; do git clone --depth 1 "https://github.com/$r.git"; done
git clone --depth 1 https://code.nym.sh/kennethnym/aris-old.git || echo "aris-old not clonable: record it"
for d in */; do echo "$d: $(ls "$d" | grep -iE '^(license|copying)' || echo 'NO LICENSE')"; done
```

- [ ] **Step 2:** For each project: license (and GPL-3.0 compatibility), what it covers, last commit, whether it agrees with our Task 6 spec (confirm/contradict our UUIDs and messages), what is missing, what is reusable.
- [ ] **Step 3:** Commit: `bash tools/check-clean-room.sh && git add docs/prior-art.md && git commit -m "docs: prior art analysis"`.

---

### Task 10: Feature inventory and STOP

**Files:**
- Create: `docs/feature-inventory.md`
- Modify: `docs/progress.md`

- [ ] **Step 1:** List **all** MyGlass features found (at least: setup/pairing, Wi-Fi, screencast, battery/info, settings, contacts, photo/video sync, Glassware, GPS, navigation, notifications/SMS/calls, timeline). For each: what it did; phone-side and Glass-side classes (`APK@version → class`); messages (link to `docs/protocol/messages.md`); cloud dependency (yes/no, link to `cloud-dependencies.md`); how we rebuild it (original protocol / bridge app / not rebuildable + alternative).
- [ ] **Step 2:** Add a "Verifiability without hardware" section: for each feature, whether it can be verified with the Task 4 approach.
- [ ] **Step 3:** Update `docs/progress.md`: Phases 1–2 done, open issues.
- [ ] **Step 4:** Commit and push

```bash
bash tools/check-clean-room.sh && git add docs && git commit -m "docs: MyGlass feature inventory (phase 2 complete)" && git push
```

- [ ] **Step 5: STOP.** Present the summary of `docs/feature-inventory.md` to the user and wait for confirmation before Phase 3.
