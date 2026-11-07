# Third-party notices

OpenMyGlass is licensed under GPL-3.0 (see [`LICENSE`](LICENSE)). The phone app and the Glass app include or download the components below, each under its own license. Builds that are distributed must ship this file, together with the license texts referenced here.

## Bundled in the phone app (`android/app`)

| Component | Version | License | Notes |
|---|---|---|---|
| [Vosk Android](https://github.com/alphacep/vosk-api) (`com.alphacephei:vosk-android`) | 0.3.75 | Apache-2.0 | Offline speech recognition. Ships `libvosk.so`, which is built on [Kaldi](https://github.com/kaldi-asr/kaldi) (Apache-2.0) and its numeric libraries (OpenFst, Apache-2.0; OpenBLAS/CLAPACK, BSD-3-Clause) **[component list per upstream build, not verified from the binary]** |
| [JNA](https://github.com/java-native-access/jna) (`net.java.dev.jna:jna`, aar) | 5.18.1 | Apache-2.0 (chosen from its LGPL-2.1 / Apache-2.0 dual license) | Ships `libjnidispatch.so` |
| [OsmAnd AIDL API](https://github.com/osmandapp/OsmAnd) (`android/osmand-api`) | vendored | GPL-3.0 | See [`android/osmand-api/NOTICE.md`](android/osmand-api/NOTICE.md) |
| [Protocol Buffers Java Lite](https://github.com/protocolbuffers/protobuf) | 4.36.2 | BSD-3-Clause | Also in the Glass app |
| [kotlinx.coroutines](https://github.com/Kotlin/kotlinx.coroutines) | 1.11.0 | Apache-2.0 | |
| [Kotlin standard library](https://github.com/JetBrains/kotlin) | 2.4.21 | Apache-2.0 | |
| [AndroidX Annotation](https://developer.android.com/jetpack/androidx/releases/annotation) | 1.11.0 | Apache-2.0 | |

## Bundled in the new companion app (`android/companion`)

| Component | Version | License | Notes |
|---|---|---|---|
| [JNA](https://github.com/java-native-access/jna) (`net.java.dev.jna:jna`, aar) | 5.18.1 | Apache-2.0 (chosen from Apache-2.0 / LGPL-2.1) | Loads the Rust core |
| [UniFFI](https://github.com/mozilla/uniffi-rs) (Rust crate and generated bindings) | 0.32.2 | MPL-2.0 | Binding layer of `rust/crates/omg-ffi` |
| Kotlin standard library | 2.4.21 | Apache-2.0 | |

## Downloaded at runtime (only when the owner asks, in settings)

| Model | License | Source |
|---|---|---|
| `vosk-model-small-it-0.22` (Italian, 48 MB) | Apache-2.0 | <https://alphacephei.com/vosk/models> |
| `vosk-model-small-en-us-0.15` (English, 40 MB) | Apache-2.0 | <https://alphacephei.com/vosk/models> |

## Glass app (`glass-xe/app`)

- Protocol Buffers Java Lite (BSD-3-Clause).
- The `glass` flavor compiles against Google's GDK (`gdk.jar`, under the GDK license), which is **not** included in the APK or this repository (`compileOnly`). See [`glass-xe/README.md`](glass-xe/README.md).

## Cross-platform companion core (`rust/`)

The Rust workspace links the crates below (and their transitive dependencies). All are
permissively licensed and compatible with GPL-3.0. Exact versions are pinned in
[`rust/Cargo.lock`](rust/Cargo.lock).

| Crate | License | Role |
|---|---|---|
| [serde](https://github.com/serde-rs/serde) / serde_json | MIT OR Apache-2.0 | Message (de)serialization |
| [postcard](https://github.com/jamesmunns/postcard) | MIT OR Apache-2.0 | Compact wire encoding |
| [tokio](https://github.com/tokio-rs/tokio) / tokio-util | MIT | Async runtime + framed codec |
| [futures](https://github.com/rust-lang/futures-rs) | MIT OR Apache-2.0 | Async sink/stream helpers |
| [bytes](https://github.com/tokio-rs/bytes) | MIT | Frame buffers |
| [async-trait](https://github.com/dtolnay/async-trait) | MIT OR Apache-2.0 | Async `Transport` trait |
| [ed25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek) | BSD-3-Clause | Identity signatures (pairing/auth) |
| [curve25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek) | BSD-3-Clause | Elliptic-curve backend |
| [getrandom](https://github.com/rust-random/getrandom) / rand_core | MIT OR Apache-2.0 | OS randomness (keys, nonces) |
| [base64](https://github.com/marshallpierce/rust-base64) | MIT OR Apache-2.0 | Key/nonce/bootstrap encoding |
| [sha2](https://github.com/RustCrypto/hashes) | MIT OR Apache-2.0 | SHA-256 for note sync and app packages (already linked through ed25519-dalek) |
| [semver](https://github.com/dtolnay/semver) | MIT OR Apache-2.0 | App versions in repository manifests |
| [libc](https://github.com/rust-lang/libc) | MIT OR Apache-2.0 | File locks for the Applications stores (Unix only) |
| [qrcode](https://github.com/kennytm/qrcode-rust) | MIT OR Apache-2.0 | Wi-Fi bootstrap QR rendering |
| [clap](https://github.com/clap-rs/clap) | MIT OR Apache-2.0 | CLI argument parsing |
| [tracing](https://github.com/tokio-rs/tracing) / tracing-subscriber | MIT | Logging |
| [thiserror](https://github.com/dtolnay/thiserror) / [anyhow](https://github.com/dtolnay/anyhow) | MIT OR Apache-2.0 | Error types |

## License texts

- Apache-2.0: <https://www.apache.org/licenses/LICENSE-2.0.txt>
- BSD-3-Clause: <https://opensource.org/license/bsd-3-clause>
- MIT: <https://opensource.org/license/mit>
- GPL-3.0: [`LICENSE`](LICENSE)
