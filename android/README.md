# android

| Module | Content |
|---|---|
| `app/` (`:android:app`) | Phone app (Kotlin, minSdk 29, targetSdk 36). Debug screen, navigation sources, connection settings (paired Glass, Bluetooth or TCP debug bridge) and `OpenMyGlassService`: a `connectedDevice` foreground service that keeps the bridge and companion links alive, with automatic reconnection. Needs the Bluetooth (`BLUETOOTH_CONNECT`) and notification permissions. |
| `osmand-api/` (`:android:osmand-api`) | OsmAnd's AIDL client API, vendored unchanged under GPLv3 — see [`osmand-api/NOTICE.md`](osmand-api/NOTICE.md). |

Build: `./gradlew :android:app:assembleDebug` (needs `local.properties` with `sdk.dir=…`, not committed).
Unit tests: `./gradlew :android:app:testDebugUnitTest`.

Third-party components bundled in the app, and the speech models it downloads, are listed in [`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md).
