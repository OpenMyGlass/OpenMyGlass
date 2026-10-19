# Navigation sources on the phone

Measured on 2026-10-19 on an Android 36 emulator (Google Play image, arm64) with:
- **OsmAnd+ 5.4.10**: F-Droid build `net.osmand.plus` 541003, offline Liechtenstein map;
- **Google Maps**: preinstalled, no sign-in.

The car route goes from Vaduz to Schaan, driven by [`tools/emulator/simulate-route.py`](../tools/emulator/simulate-route.py). Fixtures are in [`android/app/src/test/resources/maps-notifications/`](../android/app/src/test/resources/maps-notifications/): text fields and image metadata only, never the bitmaps.

## OsmAnd

### AIDL API (`net.osmand.aidl.OsmandAidlServiceV2`)

- `updateNavigationInfo(ADirectionInfo{distanceTo, turnType, isLeftSide})` fires on every routing-data update, i.e. with each location change while navigating (`OsmandAidlApi.registerForNavigationUpdates` → `RoutingHelper.addRouteDataListener`).
  - When OsmAnd has no next direction it sends `distanceTo = -1, turnType = -1`.
  - Off route, it sends `turnType = OFFR (12)` and the deviation in `distanceTo`.
- `onVoiceRouterNotify(played)` delivers the spoken prompts as text, for example "Attention, pedestrian crosswalk". These are prompts, not street names.
- Observed in our debug screen during the simulated drive: `ROUNDABOUT in 143 m` … `65 m`, with the voice text above.
- **Access control.** OsmAnd refuses every AIDL call from an app the user has not enabled. The call returns `-1`, and OsmAnd's log shows `Request AIDL API V2 for registerForNavUpdates from org.openmyglass.android enabled: false`.
  - **OsmAnd 5.4.x:** Menu → Plugins → tap "OpenMyGlass (Third-party app)". The entry appears only after our app has bound once and OsmAnd has been restarted.
  - **OsmAnd master (since 2026-10-05, not yet released):** permission groups; the app requests the `navigation` group with `startActivityForResult(Intent("net.osmand.aidl.REQUEST_PERMISSIONS").setPackage(…).putExtra("groups", arrayOf("navigation")))`.
  - `OsmAndNavigationSource.needsAuthorization` reports this state, and the debug screen explains it or offers the request.
- **Screenshot API** (`getMapScreenshot`) works only while OsmAnd's map is on screen, so it is not usable for the minimap with the phone in a pocket.

### Notification

OsmAnd's navigation notification has category `navigation`, actions "Stop" and "Pause", and a `BigTextStyle`. **It carries what the AIDL API lacks:**

| Extra | Example |
|---|---|
| `android.title` | `2.7 km • Turn right and go` |
| `android.bigText` | `Turn right and go Obergass 60 m\n2.9 km • 5 min • 8:17 PM • 0 km/h` |
| `android.largeIcon` | maneuver icon (`Icon`, bitmap type) |

The street name ("Obergass"), the remaining distance and time, and the ETA are all in `bigText`. Combining AIDL (maneuver type and distance, structured) with the notification (street, ETA) gives a complete `NavUpdate`.

## Google Maps

- **Notification permission.** Maps posts its navigation notification only if it holds `POST_NOTIFICATIONS`, which a fresh install on Android 13+ does not have until the user allows it. Our app must check for this and ask the user to enable Maps notifications.
- The notification is posted and updated while navigating, also with Maps in the background (home screen). Category `navigation`, `ongoing = true`, one action "Exit navigation" / "Esci dalla navigazione".

| Extra | English (car) | Italian (walking) |
|---|---|---|
| `android.title` | `200 m` (distance to the next maneuver) | `190 m`, sometimes empty |
| `android.text` | `28 / Landstrasse` (road) | `28`, or `verso Äulestrasse` when the title is empty |
| `android.subText` | `2 min · 1.3 km · 8:33 PM ETA` | `24 min · 1,6 km · Arrivo 20:56` |
| `android.largeIcon` | maneuver icon (`Icon`, bitmap type) | same |

- **The maneuver type is not in any text field: only the icon shows it.** To recover it we can:
  1. recognize the icon with a table of icon hashes → maneuver, built from captures (the capture tool must hash `Icon` bitmaps, not only `Bitmap` extras);
  2. forward the icon image itself to Glass at runtime, never committed;
  3. or both: recognized maneuver when known, raw icon otherwise.
- The numbers use the locale's format (`1.3` vs `1,6`), the ETA label is localized (`ETA` vs `Arrivo`), and the separator is ` · `. The distance uses a non-breaking space (`200 m`).

## Android Auto

- The Play image ships only a **stub** of Android Auto (`com.google.android.projection.gearhead` `1.2.551000-stub`). The Desktop Head Unit (`extras;google;auto`, installed, native on Apple Silicon) needs the full Android Auto app, which comes from the Play Store and needs a Google account. **Not tested yet**: it waits for the owner to sign in on the emulator, or for a real phone.
- Known limitation from research: OsmAnd uses the Car App Library, and Android Auto shows sideloaded (F-Droid) builds of such apps only after they are installed from Play or Internal App Sharing.
- Open questions for the hardware/real-car checklist: do Maps keep posting the phone notification, and does OsmAnd keep firing AIDL callbacks, while projecting to the car?

## Recommendation

1. **Primary source: OsmAnd**, combining AIDL (maneuver and distance) with its notification (street, ETA). The user must enable OpenMyGlass once in OsmAnd (Plugins).
2. **Fallback: Google Maps notification**: distance from `title`, road from `text`, time/distance/ETA from `subText` (locale-aware parsing), maneuver from the icon (hash table + raw icon forwarding). The user must allow Maps notifications.
3. Next plan: the Maps notification parser and the OsmAnd notification merge, both tested against the fixtures above; hashing `Icon` bitmaps in the capture tool.
