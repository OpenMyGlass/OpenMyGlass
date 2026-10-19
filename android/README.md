# android

| Module | Content |
|---|---|
| `app/` (`:android:app`) | Phone app (Kotlin, minSdk 29, targetSdk 36). For now a debug screen and the navigation sources; the companion service and UI come in later phases. |
| `osmand-api/` (`:android:osmand-api`) | OsmAnd's AIDL client API, vendored unchanged under GPLv3 — see [`osmand-api/NOTICE.md`](osmand-api/NOTICE.md). |

Build: `./gradlew :android:app:assembleDebug` (needs `local.properties` with `sdk.dir=…`, not committed).
Unit tests: `./gradlew :android:app:testDebugUnitTest`.
