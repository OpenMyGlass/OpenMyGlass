# tools

| Script | Purpose |
|---|---|
| `check-tools.sh` | Checks that the reverse-engineering toolchain is installed |
| `check-clean-room.sh` | Fails if tracked files contain Google binaries or Google sources; run before every commit |
| `re/apk-info.sh` | Prints package, version, file hash and signing certificate of an APK |
| `re/nano2proto.py` | Reconstructs `.proto` schemas from decompiled protobuf-nano classes |
| `sim-glass/` | Simulated Glass XE24 companion server over TCP (Kotlin; `./gradlew :tools:sim-glass:run`) |
| `tests/test-*.sh` | Tests for the scripts above (`for t in tools/tests/test-*.sh; do bash $t; done`) |

adb, logcat and HCI-snoop scripts will be added with the Android app.
