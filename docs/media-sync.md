# Photo and video sync

Photos and videos taken on Glass go into the phone's gallery (`DCIM/Glass`). Spec: [`architecture.md`](architecture.md) §5.4, §6. The Bluetooth fallback and delete-after-sync come in plan 6b.

## When

- **Automatically**, only when all of these hold:
  - Glass is charging;
  - Glass is on Wi-Fi;
  - the phone is on Wi-Fi;
  - "Sync automatically while Glass is charging on Wi-Fi" is on in the phone's settings (default on).

  A run starts when these conditions become true and whenever a new photo or video is written while they hold. If charging stops, the run finishes the current file and stops.
- **On demand** with "Sync now (Wi-Fi)" in settings, whatever the charging state.
- Files on Glass are kept.

## How

1. **Status (Glass → phone):** Glass reports charging, Wi-Fi and its Wi-Fi address when they change (`GlassStatus`, from system broadcasts). New files are noticed by a `FileObserver` on the camera folder, so there is no polling.
2. **List:** the phone asks for the list (`MediaListRequest` → `MediaList`). Ids are stable per file (path, size, modification time). Media already synced are skipped; the rest are pulled oldest first.
3. **Server:** for each pull Glass opens `MediaHttpServer` on an ephemeral port (`MediaPullStart` → `MediaServer{ip, port, token}`).
   - The token is 128 random bits, sent only over the bonded bridge.
   - It closes on `MediaPullDone`, when the session ends, or after 2 minutes without a request.
4. **Transfer:** the phone downloads each file with `GET /media/<id>?token=…` into a partial file.
   - **Resume:** a broken transfer keeps the partial file and continues with `Range` next time.
   - **Check:** the received length is verified, then the whole file's SHA-256 against Glass's `X-Content-SHA256`. A mismatch discards the file, which is retried next time.
5. **Save:** a verified file is written through `MediaStore` with `IS_PENDING` (never visible half-written), into `DCIM/Glass`, then marked synced.

## Security

- **Access to media:** Glass serves media only during a pull, only to requests with that pull's token, and only files listed in its index (no paths from requests).
- **Encryption:** the transfer is plain HTTP on the local Wi-Fi; only the Wi-Fi link encryption protects it. Someone on the same network who captures traffic during a sync could see the media. TLS with a key pinned over the bridge is a possible later improvement.
- **App setting:** the phone app enables cleartext HTTP (`usesCleartextTraffic`) for this.

## Folders

| Where | Folder |
|---|---|
| Glass XE24 | `DCIM/Camera` on external storage (**to verify on hardware**) |
| `emulator` flavor | the app's own `files/media` (push test files with `adb push`) |
| Phone | `DCIM/Glass` in the gallery |

## Verified on the emulator (2026-10-20)

- **Plug-in:** a JPEG and an MP4 (made with `screenrecord`) were synced when charging was switched on (`dumpsys battery set ac 1`). Both landed in `DCIM/Glass`, not pending, with SHA-256 identical to the originals.
- **Unplugged, then plugged:** a photo added while not charging was synced only after plugging in.
- **New photo while charging:** synced by itself (FileObserver path).
- **No duplicates** in the gallery.
- **Resume:** the resume path and hash-mismatch handling are covered by unit tests against a local HTTP server (`HttpDownloadTest`).

## Not verified yet (hardware checklist)

- XE24 camera folder and file names.
- Large video sizes and Wi-Fi throughput.
- Battery and temperature while charging and serving.
- `FileObserver` on XE24 storage.
