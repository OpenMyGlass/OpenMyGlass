# Notes with attachments and deferred sync

A note is taken on the Glass: text, a voice recording, and one or more photos or a video. It stays on the Glass until the companion holds a verified copy. If the companion is away, the note waits and syncs by itself on the next link. Speech-to-text and TTS always run on the companion.

Status: protocol and core logic are done and tested against the simulated Glass and a host glassd over TCP. glassd's side is in glass-services (`docs/api.md`, "Notes"). No Glass app adds notes yet, and nothing here has run on a Glass.

Code: `rust/crates/omg-protocol/src/notes.rs` (frames), `rust/crates/omg-core/src/notes/` (logic).

## Flow

The Glass offers one note at a time, oldest first. The companion pulls.

```text
Glass                                    Companion
NoteOffer{manifest}            ->
                               <-        NoteOfferReply{Accepted{offsets}}
                               <-        NoteChunkRequest{offset, max_len}   (a small window)
NoteChunk{offset, data, sha256} ->       check, write
                               ...
                                         whole-file SHA-256, sync, rename, persist
                               <-        NoteAttachmentAck{sha256, verified}
NoteTranscriptRequest          ->        (after the recording's ack) run STT
                               <-        NoteTranscript{text}
NoteTranscriptAck              ->
                               ...       next attachment
                               <-        NoteAck{manifest_sha256}
NoteSyncStatus{pending...}     ->        (when the link comes up and on any change)
```

TTS is pulled the same way: `NoteTtsRequest` → `NoteTtsReady` → `NoteTtsChunkRequest` / `NoteTtsChunk` → `NoteTtsRelease`. It is not queued: without a link there is no speech, and the Glass has no engine to fall back on.

The live `Dictation*` frames are untouched. They remain a scaffold for reply dictation, a separate flow.

## Guarantees

| Situation | What happens |
|---|---|
| Link drops, cleanly or mid-frame | The companion syncs what it has. The next offer resumes at that byte. |
| Companion crashes | The record never claims more than was synced. The file is cut back to the last checkpoint (every 1 MiB), so at most that much is fetched again. |
| Glass crashes | The queue is on disk. Sealing is one atomic rename, so a note is there whole or not at all. |
| Corrupted chunk | Its SHA-256 fails, it is not written, the companion sends `NoteChunkReject` and fetches it again. After 8 in a row the transfer pauses. |
| Whole file does not match | Starts over, up to 3 times. The Glass re-hashes its copy. If that copy is bad, the note is `Blocked`, kept, and the owner must be told. |
| Duplicate offer | Changes nothing. A completed note gets its acknowledgements again and no bytes move. |
| Same id, different manifest | `Rejected{ManifestConflict}`. Nothing is overwritten. |
| Lost acknowledgement | The Glass repeats its offer on a timer and on the next link. |

**Nothing on the Glass is deleted until verified.** The one path to a delete requires a verified ack for every attachment whose digest matches the Glass's own manifest, plus a `NoteAck` matching the manifest digest. The companion sends those only after the file is synced, renamed and recorded. Anything else is ignored. Once that holds, the default is to delete the Glass's copy (`DeletePolicy`).

Both state machines are pure: no I/O, no clock, no randomness. They take events and `now_ms` and return ordered effects, so all of the above is unit-tested with invented time. Timers are not persisted, so a wrong Glass clock cannot park a note for good, and the queue orders by a local counter rather than by time.

## Limits

Constants in `omg_protocol::notes`, checked by both sides.

| What | Limit |
|---|---|
| Attachments | At most 1 recording, up to 32 photos and 1 video |
| Recording / photo / video | 128 MiB / 32 MiB / 2 GiB |
| One note, in total | 3 GiB |
| Note text / transcript in a frame | 32 KiB / 48 KiB (cut on a character boundary; the companion keeps it all) |
| Chunk | 48 KiB, so frames stay under the 64 KiB the Bluetooth path is designed around |

## Open owner decisions

The owner has not answered these yet. Each is one constant, so the answer is a one-line change.

| Question | Default in the code | Where |
|---|---|---|
| Delete a note from the Glass after sync? | Yes, only after the phone's verified acknowledgements | `DeletePolicy::default()` in `omg-core` |
| Automatic sync limit per attachment on a slow link | Decided for BLE: 256 KiB. Still a default: 64 MiB on Bluetooth Classic | `AUTO_SYNC_LIMIT_MODERATE`, `AUTO_SYNC_LIMIT_CONSTRAINED` in `omg-core` |
| May one note mix photos and video? | Yes | `ALLOW_PHOTOS_WITH_VIDEO` in `omg-protocol` |

glassd pins `omg-core` by commit, so a change here reaches it when the pin moves.

## Chunk size and links

The side that pulls chooses the chunk size per request, from a `LinkProfile` given by whoever owns the link. The protocol and the state machines never name a radio.

| Profile | Chunk × window | Attachments that sync on their own | Meant for |
|---|---|---|---|
| `constrained` | 1 KiB × 2 | up to 256 KiB | Bluetooth LE |
| `moderate` | 16 KiB × 2 | up to 64 MiB | Bluetooth Classic |
| `fast` | 48 KiB × 8 | any | Wi-Fi |

A small window keeps navigation and notification frames from waiting behind a long queue of chunks. The BLE numbers come from what a prototype measured on the Glass, about 3 kB/s from it: two 1 KiB chunks are under a second ahead of a frame sent during a transfer, and 256 KiB takes about 90 s. The largest chunk of 48 KiB still crosses BLE if a peer asks for it, in about 16 s. The others are starting values: RFCOMM throughput on the Glass is still unmeasured (see `media-sync.md`).

Within a note the recording goes first, then smaller files before bigger ones, so a transcript does not wait for a video. An attachment over the profile's limit is left for a faster link (`Deferred{NeedsFasterLink}`) without holding up the next note. `media-sync.md` keeps Bluetooth pulls manual for battery reasons; this limit is how notes respect that while still syncing "on their own". Asking the Glass to bring Wi-Fi up (`WifiBringUpRequest`) is not wired yet, so a waiting attachment moves when the owner connects over Wi-Fi.

## Compatibility

- New frames are appended to `Frame`; a test pins the old discriminants. `PROTOCOL_VERSION` stays 1, because `exchange_hello` rejects any mismatch and a bump would cut off every existing glassd.
- **A peer that does not know a frame closes the link**: postcard returns a decode error, and both `FrameCodec` and glassd's decoder treat that as fatal, whatever the general rule in `lib.rs` asks for. So note frames go only to a peer that advertised `capability::NOTES_SYNC`. It is deliberately outside `BACKBONE`, and a test checks that a companion without it sees none.

## On-disk layout

One directory per note under a root the platform provides: `<id>/record.json`, `att-<n>.bin`, and `att-<n>.part` while receiving. Paths are built only from the hex id and a number, so a peer cannot name a path. Durable writes are temp file, fsync, rename, fsync directory. An unreadable record is moved aside to `.corrupt-<id>` with its files, never deleted, and the rest of the queue loads.

## Speech engines

`SpeechToText` and `TextToSpeech` are the only platform traits here, implemented by the phone apps. No engine ships in the core; `NoEngine` reports `EngineUnavailable`, which reaches the Glass as the reason there is no text. Calls are blocking and always run away from the link (`SpeechJob`), since transcribing takes far longer than a heartbeat. An interrupted transcription runs again on the next link.

## Try it

```sh
cd rust
cargo run -p omg-cli -- --store /tmp/g.json note-add --notes-dir /tmp/g --voice a.wav --photo b.jpg
cargo run -p omg-cli -- --store /tmp/g.json sim-glass --notes-dir /tmp/g \
    --read-back --corrupt-chunk 2 --drop-after-chunks 5 &
cargo run -p omg-cli -- --store /tmp/c.json notes-sync --connect 127.0.0.1:7700 \
    --notes-dir /tmp/c --fake-speech --reconnect 3
```

`--fake-speech` uses placeholder engines. `--link` picks a profile; it is still TCP underneath.

## Known limits

- SHA-256 of a finished file runs on the link's thread. On a 2 GiB video that stalls the link for some seconds.
- The Glass queues chunks for reading as fast as they are requested. A bonded peer ignoring the window could grow that queue; glassd should cap it.
- Reconnecting after a dropped link is up to the frontend (the CLI's `--reconnect`). The core has no general auto-reconnect yet, as `companion-rust-core.md` says.

## Phones

The phone apps use all this through `omg-ffi`: see [`companion-mobile.md`](companion-mobile.md).
