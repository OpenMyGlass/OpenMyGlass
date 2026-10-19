# Message catalog and per-feature sequences

Schemas: [`proto/glass-xe24/Proto.proto`](proto/glass-xe24/Proto.proto) (Glass side, XE24) and
[`proto/myglass-3.5.8/Proto.proto`](proto/myglass-3.5.8/Proto.proto) (phone side, MyGlass 3.5.8),
both regenerated with `tools/re/nano2proto.py`. Every frame on the wire is one `Envelope`
(see [`transport.md`](transport.md)). Suffix convention used by Google: `C2G` = companion→Glass,
`G2C` = Glass→companion, no suffix = both directions.

Sources: `XE24` = `_re/jadx/GlassBluetooth` (`bluetooth/companion/CompanionConnection.run()`,
`CompanionSocketServer`), `MG` = `_re/jadx/phone-myglass-3.5.8`.

## Envelope fields

| # | Field | Dir | Handled by XE24? | What happens on Glass (source) |
|---|---|---|---|---|
| 1–4 | `version`, `serialNumber`, `timeMillis`, `uptimeMillis` | both | yes | header, set by `CompanionMessagingUtil.newEnvelope()` |
| 5 | `timelineItem[]` (`TimelineNano.TimelineItem`) | C2G | **yes** | inserted into the **native timeline** (`CompanionConnection.CompanionTimelineItemRunnable` → `socketServer.insertTimelineItem`); SMS/MMS items get bundle id and share target |
| 6 | `locationRequestG2C` | G2C | — | Glass asks the phone to start/stop location updates (`LocationRequest`: provider, minTime, minDistance, priority) |
| 7 | `locationMessageC2G` | C2G | **yes** | dispatched to the Glass-side clients that requested that provider (`CompanionSocketServer.handleLocationEnvelope`) |
| 8 | `messageC2G` | C2G | yes | shows a message on Glass (`showMessageFromCompanion`) |
| 9 | `timezoneC2G` | C2G | yes | sets Glass time zone (`TimeZoneUtil.updateTimeZoneByCompanion`) |
| 10 | `navigationRequestC2G` (`uri`) | C2G | **yes** | wakes the screen and fires `Intent(ACTION_VIEW, uri)` (`startNavigationFromCompanion`), so native navigation starts; route computation: see `../cloud-dependencies.md` |
| 11 | `urlG2C` | G2C | — | Glass asks the phone to open a URL |
| 12 | `setupWifiC2G` (string) | C2G | **yes** | broadcast `com.google.glass.companion.SETUP_WIFI` → `GlassHome/home/ScreenOffWifiSetupReceiver` → `util/SetupHelper.getWifiSetupInfo`. Format: `SSID\n{WPA|WEP|NONE}[\nKEY]`, **plain text** |
| 13 | `companionInfo` | both | yes | network check, Android version, logs, locale (`handleCompanionInfoEnvelope`) |
| 14 | `error` | both | yes | `ErrorType`: tethering error, Google Voice SMS errors |
| 15 | `screenshot` | both | **yes** | start/stop screencast; Glass streams `screenshotBytesG2C` = **JPEG 640×360** frames (`StreamingThread`, `HiddenSurface.screenshot`) |
| 16 | `command` | C2G | yes | `UNPAIR`, `START_DEBUG`, `STOP_DEBUG` (`handleCommand`) |
| 17 | `timelineItemResponseC2G[]` | C2G | yes | updates companion sync status of a timeline item |
| 18/19 | `apiRequestC2G` / `apiResponseG2C` | — | **no** | logged as "Companion api is not supported" |
| 20 | `motionC2G` (`MotionEvent`) | C2G | **yes** | injected as touch input (`EventInjector.sendMotionEvent`) — remote control of the Glass UI |
| 21/22 | `glassInfoRequestC2G` / `glassInfoResponseG2C` | both | **yes** | battery level, storage, device name, software version, needSetup, screen size (`handleGlassInfo`) |
| 23/24 | `inputBoxRequestG2C` / `inputBoxResponseC2G` | both | yes | Glass asks the phone to type text (e.g. Wi-Fi password); response routed by `CompanionInputBoxResponseHandler` |
| 25/26 | `glassSetupRequestC2G` / `glassSetupResponseG2C` | both | **no (XE24)** | not read anywhere on XE24 (see `handshake.md`) |
| 27 | `setupWifiRequestG2C` | G2C | — | Glass asks the phone for Wi-Fi credentials |
| 28 | `photoG2C` | G2C | — | photo sync: thumbnail, id, operation `SYNC/DELETE/NOTICE` |
| 29 | `updateCredentialRequestG2C` | G2C | — | Glass asks the phone to refresh an account credential (cloud only) |
| 30 | `androidDeviceInfo` | both | yes | SDK version, exchanged in the handshake |
| 31 | `companionDeviceInfoC2G` | C2G | yes | phone Wi-Fi/interactive/GPS state |
| 32 | `companionFeatureInfoC2G` | C2G | yes | photo sync enabled, keyboard text entry supported |
| 33 | `glassFeatureInfoG2C` | G2C | — | wallpaper feature flag (sent in the handshake) |
| 34, 38–41 | wallpaper messages | both | only with Labs flag `WP` | set/get wallpaper |
| 35/36 | `openEndedInputRequestG2C` / `…ResponseC2G` | both | yes | free-text input typed on the phone |
| 37 | `multimediaMessage` | both | only with Labs flag `MMS` | MMS |
| 42 | `mediaRemoteControlC2G` | C2G | **not in XE24 schema** | MyGlass 3.5.8 only; Glass ignores the unknown field |
| 43 | `wifiNetworks[]` (`WifiConfig`) | — | **not in XE24 schema** | MyGlass 3.5.8 only |
| 44 | `wifiStatusG2C` | G2C | **not in XE24 schema** | MyGlass 3.5.8 only |
| 45 | `notificationC2G` (`Notification`) | C2G | **received but unconsumed** | forwarded as broadcast `com.google.glass.companion.ACTION_NOTIFICATIONS_UPDATE` (permission `com.google.glass.bluetooth.permission.COMPANION`); **no receiver found** in the decompiled XE24 APKs **[unverified: odex-only APKs like GlassSystemUI not yet checked]** |
| 46 | `inputResponseC2G` | C2G | yes (XE24 only) | broadcast `com.google.glass.remoteime.IME_SERVICE` → GlassRemoteIme (phone keyboard as Glass IME) |
| 47 | `inputRequestG2C` | G2C | XE24 only | Glass asks for remote IME input |

Schema divergence: XE24 defines 45–47 (notification, remote IME); MyGlass 3.5.8 defines 42–45
(media remote, Wi-Fi list/status, notification). Both agree on 1–41 and 45. Unknown fields are
ignored by protobuf, so mixing versions does not break parsing.

## Per-feature sequences

### Device info / battery
```
C2G Envelope{glassInfoRequestC2G{requestBatteryLevel, requestStorageInfo, requestDeviceName,
                                 requestSoftwareVersion, requestScreenDimensions}}
G2C Envelope{glassInfoResponseG2C{batteryLevel, externalStorage*Bytes, deviceName,
                                  softwareVersion, glassScreenWidth/HeightPixels}}
```
Glass also pushes battery state on its own (`BatteryPowerStateReceiver`, registered after the handshake). **[unverified: exact message used for the push]**

### Location (phone GPS → Glass)
```
G2C Envelope{locationRequestG2C{type=START_LISTENING, provider, minTime, minDistance, priority}}
C2G Envelope{locationMessageC2G{type=LOCATION_CHANGED, provider, location{...}}}   (repeated)
G2C Envelope{locationRequestG2C{type=STOP_LISTENING, provider}}
```
Glass has no GPS of its own: the companion location provider on Glass relies on this. No mock-location app is needed.

### Navigation
```
C2G Envelope{navigationRequestC2G{uri="<navigation URI>"}}  → Glass: ACTION_VIEW(uri)
```
The URI scheme accepted by GlassHome navigation, and whether it needs a cloud route, is analyzed in `../glass-internals.md` and `../cloud-dependencies.md`.

### Wi-Fi setup
```
C2G Envelope{setupWifiC2G="MyNet\nWPA\nsecret"}        → Glass joins the network
G2C Envelope{setupWifiRequestG2C{ssid, encryption}}    (Glass asks for a password)
G2C Envelope{inputBoxRequestG2C{...}} / C2G inputBoxResponseC2G{response}   (password typed on phone, minor ≥ 4)
```

### Screencast
```
C2G Envelope{screenshot{startScreenshotRequestC2G=true}}
G2C Envelope{screenshot{screenshotBytesG2C=<JPEG 640×360>, isScreenOnG2C}}  (stream)
C2G Envelope{motionC2G{...}}   (optional: drive Glass with touches from the phone)
C2G Envelope{screenshot{stopScreenshotRequestC2G=true}}
```

### SMS / timeline cards
```
C2G Envelope{timelineItem[TimelineItem{...}]}       → card inserted in the native timeline
C2G Envelope{timelineItemResponseC2G[{id, syncStatus}]}
```
`TimelineItem` is defined in [`proto/glass-xe24/TimelineNano.proto`](proto/glass-xe24/TimelineNano.proto).

### Photo sync
```
G2C Envelope{photoG2C{thumbnailBytes, timestamp, photoId, op=SYNC|DELETE|NOTICE, numOfPhotosToSync}}
```
Full-resolution transfer is not on this channel **[unverified: likely via Wi-Fi/cloud, see cloud-dependencies]**.

## Error handling

- Malformed frame or I/O error → Glass logs "Error parsing Companion envelope" and closes the connection (`CompanionConnection.run`).
- `Error{type}` messages report application-level failures (tethering, Google Voice).
- No acknowledgements at the transport level. Only timeline items have an explicit response (`TimelineItemResponse.syncStatus`).
