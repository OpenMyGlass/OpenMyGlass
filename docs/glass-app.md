# Glass bridge app (`glass-xe/`)

The OpenMyGlass app that runs on Glass Explorer Edition XE24. It receives navigation from the phone over the bridge protocol [`openmyglass.v1`](../protocol/openmyglass/v1/bridge.proto) and shows it as a card.

## What it does

- **Bridge server** (`BridgeServer`): **Bluetooth RFCOMM** (secure, UUID `8a0dcb95-c455-453f-87b5-d69ddc82b743`, SDP name `OpenMyGlass Bridge`) and **TCP port 7825** for debugging, with one session across both.
  - If Bluetooth is turned off, the RFCOMM listener is reopened with a delay that grows from 5 s up to 60 s.
  - The `glass` flavor starts at boot (`BootReceiver`), so the phone can connect without opening the app; the card appears only when navigation starts.
  - One session at a time: a new connection replaces the active one, because it means the phone reconnected.
  - It answers the phone's Hello with the capabilities `nav` and `icons`.
- **Navigation card state** (`NavCardState`): applies `NavState`, `NavEnded` and `Icon` frames.
  - Requests each unknown icon once.
  - If the phone disappears without `NavEnded`, the card stays for 30 s (the phone usually reconnects and resends) and is then removed.
- **Card**: maneuver arrow, distance, street, instruction, ETA, in Glass style on a black 640×360 layout (`nav_card.xml`, RemoteViews-safe).
  - The arrows are our own vector artwork.
  - When the maneuver is unknown (Google Maps), the card shows the maneuver icon the phone forwarded.
- **Screen wake** (`WakePolicy`): wakes on the first card, on a maneuver change, and when the distance crosses below 200 m. The wake lock lasts 5 s. Never on small distance changes, never when navigation ends.

## Security note (TCP debug port)

The phone normally uses Bluetooth RFCOMM, where pairing authenticates it. The TCP debug port **7825 listens on all interfaces with no authentication**: anyone on the same Wi-Fi or hotspot can connect. On Glass it is opened **only by debuggable builds**; release builds listen on RFCOMM only.
- A connection becomes the active session only after a valid Hello within 5 s, so a stray connection (e.g. a port scan) does not drop the phone's session.
- A client that does send a Hello can replace the session and push cards.

Use it only on trusted networks; it is meant for development.

## Notifications

The same bridge session also carries phone notifications: see [`notifications.md`](notifications.md). `BridgeService` (formerly `NavLiveCardService`) owns both LiveCards.

## Flavors

| Flavor | Renderer | Target |
|---|---|---|
| `glass` | GDK **LiveCard** (`BridgeService`): published with REVEAL, updated with `setViews`, `navigate()` + wake lock per `WakePolicy`, `unpublish()` at the end. Started from the launcher (`StartActivity`). | Glass XE24, API 19 |
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
- 2026-10-20, phone foreground service (`OpenMyGlassService`, TCP mode):
  - when the Glass app is killed and restarted, the phone reconnects by itself within the next retry (3 s in the test);
  - when the phone process is killed during navigation, the system restarts the service (`START_STICKY`, about 2 s) and the card keeps updating;
  - the Glass app in the background for 60 s during navigation does not stop updates.
- Found and fixed: after a failed send, the relay dropped the session but the link stayed open, so Glass got no updates until a reconnect. A failed send now closes the link.
- Emulator caveat: Android 36 freezes background apps, and the `emulator` flavor hosts the server in an activity. Keep the Glass emulator app in front, or start it before the phone connects; on Glass the server runs in a service.

## On real Glass (not yet verified)

1. Pair Glass with the phone (Glass: Settings → Bluetooth; phone: Bluetooth settings).
2. `adb install` the `glass` flavor; open "OpenMyGlass" once, or reboot (it starts at boot).
3. On the phone: OpenMyGlass → Connection settings → choose the Glass among the paired devices, "Bridge over Bluetooth" → Save and connect. Grant the Bluetooth and notification permissions.
4. Start a navigation in OsmAnd or Google Maps.

See the hardware checklist in [`progress.md`](progress.md).
