# Glass bridge app (`glass-xe/`)

The OpenMyGlass app that runs on Glass Explorer Edition XE24. It receives navigation from the phone over the bridge protocol [`openmyglass.v1`](../protocol/openmyglass/v1/bridge.proto) and shows it as a card.

## What it does

- **Bridge server** (`BridgeServer`): TCP on port **7825** for now; Bluetooth RFCOMM in plan 4e.
  - One session at a time: a new connection replaces the active one, because it means the phone reconnected.
  - It answers the phone's Hello with the capabilities `nav` and `icons`.
- **Navigation card state** (`NavCardState`): applies `NavState`, `NavEnded` and `Icon` frames.
  - Requests each unknown icon once.
  - If the phone disappears without `NavEnded`, the card stays for 30 s (the phone usually reconnects and resends) and is then removed.
- **Card**: maneuver arrow, distance, street, instruction, ETA, in Glass style on a black 640×360 layout (`nav_card.xml`, RemoteViews-safe).
  - The arrows are our own vector artwork.
  - When the maneuver is unknown (Google Maps), the card shows the maneuver icon the phone forwarded.
- **Screen wake** (`WakePolicy`): wakes on the first card, on a maneuver change, and when the distance crosses below 200 m. The wake lock lasts 5 s. Never on small distance changes, never when navigation ends.

## Security note (interim TCP bridge)

Until Bluetooth RFCOMM lands (plan 4e, where bonding authenticates the phone), the bridge listens on **TCP port 7825 on all interfaces with no authentication**. Anyone on the same Wi-Fi or hotspot can connect.
- A connection becomes the active session only after a valid Hello within 5 s, so a stray connection (e.g. a port scan) does not drop the phone's session.
- A client that does send a Hello can replace the session and push cards.

Use it only on trusted networks; it is meant for development.

## Flavors

| Flavor | Renderer | Target |
|---|---|---|
| `glass` | GDK **LiveCard** (`NavLiveCardService`): published with REVEAL, updated with `setViews`, `navigate()` + wake lock per `WakePolicy`, `unpublish()` at the end. Started from the launcher (`StartActivity`). | Glass XE24, API 19 |
| `emulator` | `EmulatorCardActivity`: the same layout in the largest 16:9 area of the screen, with the bridge server inside the activity. | Any Android emulator (API 21+) |

## GDK and licensing

See [`glass-xe/README.md`](../glass-xe/README.md).
- The `glass` flavor compiles against Google's `gdk.jar`, installed with sdkmanager after accepting the GDK license. The jar is **never committed, decompiled or disassembled**.
- It also compiles against OpenMyGlass's own stubs (`-PgdkStubs=true`), written from the public reference docs. Both builds are checked.

## Testing without Glass (emulator, 2026-10-19)

On one Android 36 emulator, the phone app and the `emulator` flavor run side by side:

```bash
./gradlew :android:app:assembleDebug :glass-xe:app:assembleEmulatorDebug
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
adb install -r glass-xe/app/build/outputs/apk/emulator/debug/app-emulator-debug.apk
# open "OpenMyGlass (emulator)", then in the phone app: Preset "Glass app on this device" → Connect bridge
python3 -I tools/emulator/simulate-route.py tools/emulator/routes-vaduz-schaan.csv --speed-kmh 40
```

Observed:
- **OsmAnd:** roundabout arrow, `31 m`, `Prendere la 2 uscita e proseguire Herrengasse verso Feldkirch, Schaan`, `ETA 23:43`.
- **Google Maps:** Maps' own straight-arrow icon, `Bangarten`, `ETA 23:43`.
- The card disappears when navigation stops.
- Found and fixed: rotating the emulator recreated the activity and dropped the bridge session (now `configChanges`).
- Gap: the phone does not reconnect the bridge by itself yet (plan 4e: foreground service + reconnection).

## On real Glass (not yet verified)

See the hardware checklist in [`progress.md`](progress.md): install with `adb install`, start "OpenMyGlass", connect the phone over Wi-Fi to the Glass IP on port 7825 (until RFCOMM lands), and check LiveCard publishing, wake behavior and battery.
