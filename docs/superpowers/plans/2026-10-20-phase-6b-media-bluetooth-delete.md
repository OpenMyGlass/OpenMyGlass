# Phase 6b: Media over Bluetooth and delete-after-sync — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:**
- **On-demand sync without Wi-Fi:** when Glass or the phone is not on Wi-Fi, "Sync now" pulls media over the Bluetooth bridge, after the owner has seen a speed warning.
- **Delete after sync (optional, default off):** Glass deletes a file only after the phone confirms it is saved and verified.

**Architecture:**
- **Bridge (`openmyglass.v1`, additive):**
  - Phone → Glass: `MediaChunkRequest{id, offset, length}`, `MediaDelete{id, sha256}`.
  - Glass → phone: `MediaChunk{id, offset, data, total, sha256}`. `data` is at most 48 KiB, so the frame stays under 64 KiB; `sha256` is in the first chunk. Then `MediaDeleted{id, ok}`.
- **Glass:** `MediaHandler` answers chunk requests from the index: indexed ids only, offsets bounded by the file size, reads done off the session's write lock where possible. It deletes a file only if `MediaDelete` carries the same SHA-256 the index computed for the current file version.
- **Phone:**
  - core `MediaSync` gets a transport choice per run. Over Wi-Fi the `MediaServer` path stays as it is. Over Bluetooth, used for an on-demand run when `MediaServerUnavailable` arrives or the phone is not on Wi-Fi, it requests chunks one at a time through a `ChunkDownloader` that appends to the same resumable partial file, then checks the SHA-256.
  - The owner confirms in settings ("Sync now over Bluetooth (slow)"). Automatic runs stay Wi-Fi only.
  - When delete-after-sync is on, each saved item is followed by `MediaDelete`.

**Spec:** [`docs/architecture.md`](../../architecture.md) §5.4 (on demand: over Wi-Fi when available, otherwise Bluetooth with a speed warning; delete only if the owner enables it), §6, §10; [`docs/media-sync.md`](../../media-sync.md).

## Global Constraints
- **Bluetooth:** used only on demand, never automatic (battery and the link's bandwidth for navigation and notifications). One chunk in flight at a time.
- **Deletion:** never without the owner's setting, never before the gallery write succeeded, never if the file changed (hash check on Glass).
- English only; no Claude attribution; clean-room guard after `git add`.

## Review Focus
1. **Link drop mid-chunk:** the partial file is kept and the next run resumes from its length. Nothing is saved half-written.
2. **File changed or deleted on Glass between chunks:** the hash check fails and nothing is saved. A delete for a changed file is refused.
3. **Chunk request abuse:** an unknown id, a negative offset or a huge length is answered with an empty error chunk. No data outside the file.
4. **Delete-after-sync off (default):** never sends `MediaDelete`.
5. **Navigation and notifications keep flowing during a Bluetooth pull:** chunks are requested one at a time, so other frames interleave.

### Task 1: Protocol + core `MediaSync` Bluetooth path and delete-after-sync (fake chunk source in tests)
### Task 2: Glass `MediaHandler` chunk serving and verified delete (tests: bounds, unknown id, hash-guarded delete)
### Task 3: Phone `ChunkDownloader` (resume, hash) + settings (Bluetooth sync with warning, delete after sync) + wiring
### Task 4: End to end on the emulator
- Turn Wi-Fi off on the "Glass" side by simulating `wifi=false`: the emulator flavor gets a debug toggle that forces "no Wi-Fi".
- Sync now over Bluetooth (TCP bridge on the emulator); the file reaches the gallery.
- With delete-after-sync on, the file is removed from the Glass folder after being saved.

Then docs.
