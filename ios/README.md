# iOS companion

A thin SwiftUI shell around the Rust core (`rust/crates/omg-ffi`), like `android/companion`. Phase 1 connects over TCP and shows the link and the Glass state; Bluetooth and speech are not there yet.

```sh
./build-core.sh          # XCFramework and generated OmgCore.swift into build/
xcodegen generate        # writes OpenMyGlassCompanion.xcodeproj (not committed)
xcodebuild -scheme OpenMyGlassCompanion -destination 'generic/platform=iOS Simulator' build
```

`build-core.sh` needs the `aarch64-apple-ios` and `aarch64-apple-ios-sim` Rust targets on a toolchain that can build the workspace lockfile (`RUSTUP_TOOLCHAIN=1.96.0`, for instance). The core is built for arm64 only, so the simulator build is arm64 too.

To try it, run the simulated Glass on the Mac, which the simulator reaches at `127.0.0.1`:

```sh
cd ../rust
cargo run -p omg-cli -- sim-glass --store /tmp/sim/state.json --notes-dir /tmp/sim/notes
cargo run -p omg-cli -- note-add --store /tmp/sim/state.json --notes-dir /tmp/sim/notes --text "Hello"
```

Connect to `127.0.0.1:7700` with "Allow pairing" on the first time. Later links reuse the bond, and a note added with `note-add` arrives and is counted.

Not there yet: Bluetooth (Core Bluetooth around `BleWritePacer`), speech engines (the stubs answer `Unavailable`), the notes screen, settings, Unpair, and background operation. None of this has run on a device.
