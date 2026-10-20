# Phase 6a: Photo and video sync over Wi-Fi — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Photos and videos taken on Glass reach the phone's gallery. Sync runs **automatically when Glass is charging and on Wi-Fi** (and the phone is on Wi-Fi). It can also be run **on demand** from the phone (over Wi-Fi). Every transfer is resumable and verified by SHA-256. Files on Glass are kept. The Bluetooth fallback and delete-after-sync are plan 6b.

**Architecture:**
- **Glass (API 19):**
  - `MediaIndex` scans the camera folder: XE24 `DCIM/Camera`, to verify on hardware; the emulator flavor uses its own files dir. A `FileObserver` reports new files, with no polling. Ids are stable: a hash of the path, size and mtime.
  - `GlassStatus` reports charging, Wi-Fi and IP when they change (sticky broadcasts, event-driven).
  - `MediaHttpServer` is opened only while the phone pulls. It serves `GET /media/<id>` with `Range` support, the `X-Content-SHA256` header and a one-time token that only travels over the bonded bridge. It closes on `MediaPullDone`, or after 2 minutes idle.
- **Bridge (`openmyglass.v1`, additive):**
  - Glass → phone: `GlassStatus{charging, wifi, ip}`, `MediaList{items[{id, name, video, size, taken_at_ms}]}` (on request and when new media appear), `MediaServer{ip, port, token}` or `MediaServerUnavailable{reason}`.
  - Phone → Glass: `MediaListRequest{}`, `MediaPullStart{}`, `MediaPullDone{}`.
- **Phone:**
  - core `MediaSync`: decides when to auto-sync (`SyncPolicy`), what to pull (ids not yet synced, oldest first) and runs downloads through a `MediaDownloader` interface. Media are pulled one at a time; on errors it retries the next time the conditions hold.
  - app `HttpMediaDownloader`: Range resume from a partial file in the cache, SHA-256 checked before saving.
  - app `GallerySaver`: `MediaStore` with `IS_PENDING`, folder `DCIM/Glass`.
  - app `SyncedMedia`: synced ids kept in app storage.
  - Settings: auto-sync on/off (default on) and "Sync now".

**Spec:** [`docs/architecture.md`](../../architecture.md) §5.4 (auto-sync on charging + Wi-Fi, LAN transfer, resumable, hash, gallery, keep files on Glass, on demand), §6 (Wi-Fi media only under charging + Wi-Fi; no polling), §10.

## Global Constraints
- **Battery (§6):** auto-sync only while Glass is charging and on Wi-Fi. The HTTP server is open only during a pull. No polling.
- **Security:** the media server accepts only requests carrying the per-pull random token (128-bit) sent over the bonded bridge, serves only indexed ids (no paths from requests), and listens only while pulling.
- **Privacy:** media go only to the owner's phone gallery. Nothing is uploaded elsewhere.
- English only; no Claude attribution; clean-room guard after `git add`.

## Review Focus
1. **Interrupted transfer** (Wi-Fi drop, Glass unplugged mid-video): the partial file is kept, the next pull resumes with `Range`, and nothing half-written reaches the gallery.
2. **Hash mismatch or truncated file:** the item is not saved, is not marked synced, and is retried.
3. **Media appearing while a pull runs:** picked up by the next list, never duplicated in the gallery.
4. **Wrong or missing token, path tricks** (`/media/../`, unknown id): 403/404, no file outside the index served.
5. **Conditions change during sync** (charging stops): the current item finishes, then auto-sync stops. On-demand sync is not affected.

### Task 1: Protocol + core `SyncPolicy` and `MediaSync` (fake downloader and fake bridge in tests)
### Task 2: Glass: `MediaIndex`, `MediaHttpServer` (JVM tests over real sockets: range, token, unknown id, traversal), `GlassStatus`, media handler on the bridge
### Task 3: Phone: `HttpMediaDownloader` (resume, hash; JVM test against a local server), `GallerySaver`, `SyncedMedia`, wiring into `BridgeManager`/`PhoneHub`/service, settings and "Sync now"
### Task 4: End to end on the emulator
- Push a JPEG and an MP4 into the Glass emulator app's media dir.
- Simulate charging with `dumpsys battery set ac 1`; the emulator is on Wi-Fi.
- Auto-sync puts both in the phone gallery (`content query`), with hashes checked.
- Interrupt one transfer and check that it resumes.
### Task 5: Docs (`docs/media-sync.md`, progress, hardware checklist: XE24 camera folder, video sizes, Wi-Fi throughput, battery while charging)
