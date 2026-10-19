# glass-xe

OpenMyGlass bridge app for **Glass Explorer Edition XE24** (Android 4.4.4, API 19).

| Module | Content |
|---|---|
| `app/` (`:glass-xe:app`) | Bridge server, navigation card state and renderers. Flavors: **`glass`** (GDK LiveCard, minSdk/targetSdk 19, for the glasses) and **`emulator`** (plain 640×360 Activity, no GDK, for testing on a normal Android emulator). |
| `gdk-stubs/` (`:glass-xe:gdk-stubs`) | OpenMyGlass's own compile-only stubs of the few GDK classes we use, written from the public GDK reference documentation. |

## The GDK

The `glass` flavor compiles against the GDK (`compileOnly`; the real classes come from the Glass system at runtime). Two options:

- **Google's `gdk.jar`** (default): `sdkmanager --include_obsolete "add-ons;addon-google_gdk-google-19"`. This means accepting Google's GDK license, which forbids redistributing, modifying, decompiling or disassembling the jar. It is read from `$ANDROID_HOME/add-ons/addon-google_gdk-google-19/libs/gdk.jar` (or `-PgdkJar=<path>`) and is **never committed** (`tools/check-clean-room.sh` rejects tracked jars).
- **OpenMyGlass stubs**: `-PgdkStubs=true`. No Google license is involved. The stubs declare only what the docs publish, with each class citing its reference page.

Both must compile; that is how stub compatibility is checked.

```bash
./gradlew :glass-xe:app:assembleGlassDebug                    # with Google's gdk.jar
./gradlew -PgdkStubs=true :glass-xe:app:assembleGlassDebug    # with our stubs
./gradlew :glass-xe:app:assembleEmulatorDebug                 # no GDK
./gradlew :glass-xe:app:testGlassDebugUnitTest                # JVM tests
```
