# The companion on phones

A shared Rust core with thin native frontends, on Android and iOS from the start. This page covers how a phone app uses the core. The core itself is in [`companion-rust-core.md`](companion-rust-core.md), and notes in [`companion-notes-sync.md`](companion-notes-sync.md).

Status: `omg-ffi` is tested on the host against the simulated Glass over TCP, and the Android app has run on a phone against a Glass over Wi-Fi. It has run on no iOS device, and Swift has only had its bindings generated and the Rust side compile-checked for `aarch64-apple-ios`.

## The binding layer: UniFFI in `omg-ffi`

The plan held. `rust/crates/omg-ffi` holds one Rust definition, and UniFFI generates Kotlin and Swift from it, including the foreign implementations of the speech engine traits. UniFFI is MPL-2.0, which is compatible with this project's GPL-3.0, and is pinned exactly. `omg-ffi` builds without tokio, like `omg-core`, and CI checks that.

The one change from the first sketch: raw socket bytes cross the boundary, not encoded frames. Framing then lives in Rust once, and an app needs no knowledge of it.

Generate the sources with:

```sh
cd rust
cargo build -p omg-ffi
cargo run -p omg-ffi --features cli --bin uniffi-bindgen -- generate \
    --library target/debug/libomg_ffi.so --language kotlin --language swift --out-dir /tmp/gen
```

## Rules that keep iOS first-class

- **The app owns the socket.** The core never opens one and nothing in its API names a radio. Android may hand it an RFCOMM `BluetoothSocket`, iOS a BLE or `Network.framework` stream. iOS has no Bluetooth Classic for this use, without MFi hardware.
- **A link describes itself only as a `LinkProfile`**: chunk size, window, and the largest attachment that syncs on its own. The app picks one for the link it opened.
- **No runtime and no callback-driven I/O.** The app calls in from threads it controls. iOS can suspend the app at any time, so nothing depends on a background thread staying alive: everything durable is on disk, and the Glass resumes on the next link.
- **Foreign callbacks are only the speech engines.** They are blocking and run on threads the core starts, so an engine can wrap a platform API by waiting on it.

Not solved by this layer: "sync on its own" on iOS depends on Core Bluetooth background modes, which need testing on a device.

## Using it

Two threads per link, and one more while the app is open:

```text
reader:  loop { n = socket.read(buf); core.on_bytes(buf[..n]); wake(pump) }
pump:    loop { wait(wake, at most 250 ms); o = core.pump();
                socket.write(o.outgoing); show(o.events); if o.close_link { break } }
then:    socket.close(); core.link_down()
poller:  loop { sleep(500 ms); show(core.poll()) }
```

A blocked write never stops reading, which matters on small Bluetooth buffers, where two peers could otherwise each wait for the other to read. Bytes leave in the order the core produced them, whichever thread called it. `poll()` exists because nothing pumps with no link, yet a transcript or a note read aloud can finish then. While a link is up it returns nothing, so every event of a link reaches the pump, in order and with the link it came from. It leaves what `pump()` must write alone, so it needs no coordination with the link's threads.

**No call belongs on a UI thread.** A call waits for the core's lock, which the reader holds while it syncs a file or hashes a whole attachment, for seconds on a big video. The Android screen makes all its calls on one background thread of its own and draws what comes back. That thread is for the screen only: the link and the poller call the core from their own.

**One core per directory, for the life of the process.** A second one on the same directory is refused while the first is alive. The guard is per process: an iOS app extension sharing the directory through an App Group is not stopped by it, so the app must not let one open a core there. On Android, `CompanionProcess` holds the core, the link and the draft, so a screen being recreated or closed ends none of them.

For a UI: `snapshot()` is the link and what the Glass last reported, `notes()` is the list, and `CoreEvent`s say when to read them again.

- **Settings:** `change_settings` sends only the fields set. The Glass clamps values and answers with the full mirror as `SettingsChanged`. Radios have `set_radio` and `set_airplane_mode`. A field the Glass does not report stays `None`, and the UI hides it: today that is `brightness`, which glassd does not have.
- **Pairing:** `link_up(profile, allow_pairing)`. Set `allow_pairing` only when the owner asked to pair. `unpair(link_id)` needs the running link the owner tapped it on, from `snapshot().link_id`, past its handshake, and changes nothing otherwise, so an old Unpair never ends a newer link. It forgets the bond and only queues the Unpair frame: if the link dies before the pump writes it, or the write or flush fails, the Glass keeps its bond. Tapping Unpair and then Connect is one way: the old socket is already closed for the new link while the core still holds the old link. Afterwards the app stays down until the owner asks for a link, or it would pair again by itself. That relies on the reconnect's shortest wait, 1 s (`omg_core::reconnect::MIN_WAIT_MS`), being longer than the app takes to stop its supervisor after `unpair`.
- **Notes from the Glass** arrive by themselves. Attachment paths are set once a file is verified.
- **Notes on the phone** (`add_phone_note`) are stored on the phone only. The protocol syncs in one direction, from Glass to phone. A transcription cut short by the app stopping runs again on the next start.
- **A note's state:** `NoteInfo` says whether it is `transcribing`, what it is `waiting` for, and whether it is `deletable` or `speakable`. `snapshot().receiving` is the note being pulled, and `CoreEvent::NoteProgress` says when to read it, about once a second.
- **Reading a note aloud** (`speak_note`) needs no link. The core picks the text, its own and then what was heard, and writes a WAV file a piece at a time. The app gets `NoteSpeechReady{path}` or `NoteSpeechFailed{reason}`, each with the `request_id` it passed, and only plays the file. `stop_speaking` calls the TTS engine's `cancel`. In a narrow race, a stop just as reading ends can cancel the next TTS job instead, which may be one the Glass asked for; it fails, and the Glass may ask again. Speech-to-text has no cancel and always runs to its end.
- **Asking for a transcript again** (`retranscribe_note`) is for a recording with no text, for instance once there is an engine. Failures are not retried by themselves. A transcription that was interrupted is, up to 3 starts, counted on disk just before the engine is called, so notes waiting behind it are not charged, so one that takes the app down cannot do so on every launch. After that the note says so, and the owner can ask again.
- **Deleting** (`delete_note`) removes a note and its files from this phone. A note from the Glass must be all here: the Glass would offer an incomplete one again. If the Glass never got the acknowledgement of a note deleted here, it offers it again and the note comes back, which is the safe way for that to go wrong.

## The BLE link

BLE is the everyday link; Wi-Fi is for photos and video, or when Bluetooth fails. The Glass end is `omg-bled` in glass-services (its `docs/ble-link.md`), and the design and measurements are in [`plans/2026-11-05-ble-companion-link.md`](plans/2026-11-05-ble-companion-link.md).

The stream is the same bytes as TCP: each write and notification is the next run, with no header. **Status: tested on the host with a simulated radio. It has not run against a Glass.** The prototype it comes from (`tools/ble-proto` on branch `ble-proto`) did: MTU 247, 27-byte PDUs, about 6 kB/s to the Glass, 3.6 kB/s from it, no loss, and stalls of up to 3.6 s whose cause is unknown.

What an app does, in order: scan by `ble_service().service_uuid`, connect, ask for the MTU, discover, subscribe to `tx`. The Glass starts its session on the subscription, and its first bytes wait for the core. Then:

```text
write(bytes):  pacer.push(bytes)
               loop { match pacer.next_step(now) { Write => send one run, and wait for the stack to take it
                                                    Wait(ms) => sleep(ms)
                                                    Idle => return } }
notification:  queue the value for the reader thread
disconnect:    end of stream, then core.link_down(); open again
```

- **Writes are paced in the core** (`omg_core::ble::WritePacer`, `BleWritePacer` here), at 3 kB/s, because Android queued unpaced writes for up to 13 s ahead of anything sent later. The blocking write is the core's backpressure. Reading never waits for it.
- **A reconnect is a new session:** a new handshake, nothing carried over. A bond makes it automatic, and note sync resumes from disk.
- **Trying again** waits by `omg_core::reconnect`: it doubles up to 30 s, and only a link that lasted that long or the owner asking resets it, because each attempt is a scan, which Android throttles, and wakes the Glass.
- **The last frame** of a link has no acknowledgement. Before a close the core asked for, `BleLink.flush` waits for the stack to take the last run and gives the radio half a second. That makes it likely, not certain, that an `Unpair` arrives.
- **TODO, not built: make Unpair reliable.** Two ways, not exclusive:
  - *An acknowledgement.* The Glass answers `Unpair` with an `UnpairAck` once it has forgotten the phone. The phone forgets only on the ack, and keeps a pending Unpair on disk until then, sending it again on the next link, which the old bond still authenticates. A pending Unpair is shown to the owner. This needs a new frame and capability, and glassd to send it.
  - *Tolerance on the Glass.* A Glass still bonded to a phone that no longer knows it sees that phone try to pair afresh, or not at all. It could offer, on its own screen, to forget the old bond. This needs no protocol change, and covers a phone that was reset or lost too.
- **A pump waits for paced writes**, a second or so at most with two 1 KiB chunks in flight. The core's only timers are the 30 s note response deadlines, on the wall clock, so a late pump delays a retry and never causes one.
- **The radio warning** covers BLE only: before turning Bluetooth off or airplane mode on, the app warns when the link it would end is over Bluetooth. Over TCP it does not warn when turning the Glass's Wi-Fi off or airplane mode on, because the app cannot tell a link over Wi-Fi from one over USB (`adb reverse`). Still to do.
- **Over the limit:** the profile sets 1 KiB chunks, 2 in flight, and syncs attachments up to 256 KiB by itself. A bigger one waits for a link with no limit (`Deferred{NeedsFasterLink}`). See `companion-notes-sync.md`.
- **iOS** writes the same few lines around `CBPeripheral`, with `canSendWriteWithoutResponse` in place of the Android callback. `BleLink.kt` holds no logic it needs to copy beyond this.

On Android, `BleLink.kt` is the only radio code. It asks for `BLUETOOTH_SCAN` and `BLUETOOTH_CONNECT` from Android 12, and fine location before that. A bonded phone looks only for the address that passed a handshake, comparing it in the scan callback, because the Glass's address is random and a filter on an address may assume a public one.

### Staying alive in the background

Not built yet. The classes are ready for it: `BleLink` and `LinkController` take the application context and hold no screen, and `MainActivity` only draws what the core reports. The step is a foreground service of type `connectedDevice` that owns `CompanionCore` and `LinkController`, with `FOREGROUND_SERVICE_CONNECTED_DEVICE`, a notification, and the Activity binding to it. `CompanionDeviceManager` can add presence and battery exemptions, and an OEM battery manager may still stop it.

### Trying it on a Glass

1. Boot an image with `glassd --radio on --companion-ble on` and `omg-bled` beside it: `docs/ble-link.md` in glass-services.
2. `./gradlew :android:companion:assembleDebug`, install, open the app, press **Bluetooth**, and grant the permissions.
3. The status goes from "Looking for the Glass over Bluetooth" to "Connected to ... over Bluetooth". The Glass pairs on this first link if it has no bond yet: **pairing is open**, as on every link.
4. Change a setting, take a note on the Glass, and see it arrive. Turn Bluetooth off and on, or walk away: the status returns by itself.

## The Android app (`android/companion`)

A first app, with no design work yet: every screen is built in code, the text is English only, and all logic is in the core. For notes it records a voice note, attaches photos and a video, shows photos full size and plays recordings and video in the app, reads notes aloud, transcribes again and deletes. None of this has run on a phone yet. `./gradlew :android:companion:assembleDebug` builds the core for `arm64-v8a` and generates the bindings, so it needs `cargo` and `rustup target add aarch64-linux-android`. Other ABIs are one line in `RustAbi`.

To try it without a Glass, run a host glassd with its hardware back ends off (`--display log --glasshub none --radio off --power off` and so on), then `adb reverse tcp:7700 tcp:<port>`, and Connect to the default `127.0.0.1:7700`. `crates/glass-api/examples/note-add.rs` in glass-services adds notes to it.

Known limits:

- RFCOMM is not there. It would be one more `ByteLink`, on Android only.
- The link lives as long as the app's process: there is no foreground service, so nothing syncs while Android has stopped the app. See the BLE section below.
- Speech uses the platform's engines, and a phone may have none. On the test phone (crDroid, no Google apps) neither exists, so notes say there is no transcript, and `AndroidTts` and `AndroidStt` have never produced anything. The fix is to bundle engines. Vosk is already used in `android/app`.
- A note cannot be sent to the Glass, and a note that has not finished arriving cannot be discarded from here.
- A draft lives only as long as the app's process: closing the screen keeps it, and Android killing the process loses it.
- The note list builds every note up front, so a very long list will be slow.

## Speech

`SpeechToTextEngine` and `TextToSpeechEngine` are implemented by each app and always run on the companion. Files do not cross the boundary: an engine gets a path and returns text, or returns PCM. An engine that has no answer returns `Unavailable`, which reaches the Glass as the reason there is no text. Text over 3000 bytes goes to the TTS engine in pieces, cut after a sentence or a word, because Android takes 4000 characters at most. The app gets one audio. One thread runs speech-to-text and one runs text-to-speech, so engines see one request at a time. An engine answering `Busy` is tried again 3 times, a second apart, before the note says so.
