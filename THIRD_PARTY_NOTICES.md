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

## Downloaded at runtime (only when the owner asks, in settings)

| Model | License | Source |
|---|---|---|
| `vosk-model-small-it-0.22` (Italian, 48 MB) | Apache-2.0 | <https://alphacephei.com/vosk/models> |
| `vosk-model-small-en-us-0.15` (English, 40 MB) | Apache-2.0 | <https://alphacephei.com/vosk/models> |

## Glass app (`glass-xe/app`)

- Protocol Buffers Java Lite (BSD-3-Clause).
- The `glass` flavor compiles against Google's GDK (`gdk.jar`, under the GDK license), which is **not** included in the APK or this repository (`compileOnly`). See [`glass-xe/README.md`](glass-xe/README.md).

## License texts

- Apache-2.0: <https://www.apache.org/licenses/LICENSE-2.0.txt>
- BSD-3-Clause: <https://opensource.org/license/bsd-3-clause>
- GPL-3.0: [`LICENSE`](LICENSE)
