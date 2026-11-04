# The companion on phones

A shared Rust core with thin native frontends, on Android and iOS from the start. This page covers how a phone app uses the core. The core itself is in [`companion-rust-core.md`](companion-rust-core.md), and notes in [`companion-notes-sync.md`](companion-notes-sync.md).

Status: `omg-ffi` is tested on the host, against the simulated Glass over TCP. It has run on no iOS device, and Swift has only had its bindings generated and the Rust side compile-checked for `aarch64-apple-ios`.

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

Not solved by this layer: a BLE peripheral on the Glass does not exist yet, and "sync on its own" on iOS depends on Core Bluetooth background modes, which need testing on a device.

## Using it

Two threads per link:

```text
reader:  loop { n = socket.read(buf); core.on_bytes(buf[..n]); wake(pump) }
pump:    loop { wait(wake, at most 250 ms); o = core.pump();
                socket.write(o.outgoing); show(o.events); if o.close_link { break } }
then:    socket.close(); core.link_down()
```

A blocked write never stops reading, which matters on small Bluetooth buffers, where two peers could otherwise each wait for the other to read. Bytes leave in the order the core produced them, whichever thread called it.

For a UI: `snapshot()` is the link and what the Glass last reported, `notes()` is the list, and `CoreEvent`s say when to read them again.

- **Settings:** `change_settings` sends only the fields set. The Glass clamps values and answers with the full mirror as `SettingsChanged`. Radios have `set_radio` and `set_airplane_mode`. A field the Glass does not report stays `None`, and the UI hides it: today that is `brightness`, which glassd does not have.
- **Pairing:** `link_up(profile, allow_pairing)`. Set `allow_pairing` only when the owner asked to pair.
- **Notes from the Glass** arrive by themselves. Attachment paths are set once a file is verified.
- **Notes on the phone** (`add_phone_note`) are stored on the phone only. The protocol syncs in one direction, from Glass to phone.

## Speech

`SpeechToTextEngine` and `TextToSpeechEngine` are implemented by each app and always run on the companion. Files do not cross the boundary: an engine gets a path and returns text, or returns PCM. An engine that has no answer returns `Unavailable`, which reaches the Glass as the reason there is no text.
