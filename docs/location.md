# Location from the phone

Glass gets its position from the phone, not from its own GPS chip (owner decision, 2026-10-22). The phone's receiver is better, it already runs for navigation, and leaving the Glass chip off saves Glass's small battery and heat at the temple. Messages: [`bridge.proto`](../protocol/openmyglass/v1/bridge.proto) `LocationSubscribe`, `LocationUpdate`, `LocationUnavailable` (Frame fields 26-28).

This is the bridge path (our Glass app). The XE24 firmware's own path over the companion link (`locationRequestG2C` / `locationMessageC2G`, [`protocol/messages.md`](protocol/messages.md)) is separate and not implemented.

## Purpose on Glass

- **glass-services** exposes the fixes as a location feed for Glass apps.
- **Android glue** on Glass will register the feed as an Android location provider, so ordinary apps get the phone's position through `LocationManager`.
- While a Glass app wants location, Glass holds one subscription with the shortest interval and the highest accuracy any client asked for.

## Protocol

1. **Capability:** the phone lists `location` in its bridge `Hello` when it serves location. Glass does not subscribe to a phone without it.
2. **Subscribe (Glass → phone):** `LocationSubscribe{interval_ms, high_accuracy}`.
   - A new subscribe replaces the previous one.
   - `interval_ms = 0` stops it.
   - The phone raises intervals under 1000 ms to 1000 ms.
   - `high_accuracy` asks for GPS-grade fixes, which cost the phone more battery. Without it the phone uses the fused or network provider.
3. **Updates (phone → Glass):** `LocationUpdate` for each fix, only while subscribed.
   - `fix_time_ms` is the UTC time of the fix, not of sending. Glass judges staleness from it.
   - `accuracy_m = 0` means unknown.
   - Absent `altitude_m`, `bearing_deg` and `speed_mps` mean unknown.
   - `provider` is `gps`, `network` or `fused`.
   - A recent last-known fix (at most 2 minutes old) is sent right after subscribing, so Glass has a position at once.
4. **Unavailable (phone → Glass):** `LocationUnavailable{reason}`:
   - `permission`: the owner has not granted location to OpenMyGlass on the phone;
   - `disabled`: location is turned off on the phone;
   - `no-fix`: no fix within 60 s of starting. It is not sent while `permission` or `disabled` is the current report.

   The subscription stays active. The phone retries a refused start every 60 s, and fixes resume when the cause goes away. The same reason is not sent twice in a row.
5. **End:** the subscription ends with the bridge session. After a reconnect Glass subscribes again if it still wants location.

## Phone side

- **Core:** `core/…/location/LocationRelay` turns subscriptions into starts and stops of a `LocationSource`. Tests use a fake source (`LocationRelayTest`).
- **Android source:** `android/…/location/AndroidLocationSource` uses the platform `LocationManager` (no Play services).
  - High accuracy: GPS, else fused, else network.
  - Otherwise: fused (Android 12+), else network.
  - With only approximate location granted, GPS is not used.
- **Permission:** `ACCESS_FINE_LOCATION` (with `ACCESS_COARSE_LOCATION`, as Android 12+ requires), from the "Share the phone's location with Glass" button in settings.
- **Background:** the connection service also takes the `location` foreground-service type when location is granted. Then fixes keep flowing with the app in the background.
  - On Android 14+ that type is refused when the service starts from the background (at boot, sticky restart). The service then runs without it, and the phone may send `permission` or no fixes with the app in the background. Merely opening the app does not restore the type. "Save and connect" or "Share the phone's location with Glass" in settings restarts the service with it.
  - The phone does not ask for background location (`ACCESS_BACKGROUND_LOCATION`).
- **Battery:** the phone's location runs only between a subscribe and its stop or the session's end. Nothing polls.

## Privacy

- Fixes travel only over the paired bridge (bonded RFCOMM, or the debug TCP link, which is plaintext on the local network). They never reach a server.
- The phone sends fixes only while Glass holds a subscription. It neither stores nor logs them.
- Glass keeps nothing beyond the last fix, in memory. It writes no location history and drops the fix when the bridge app stops.

## Checklist on Glass (not run: no hardware)

- [ ] Subscribe from glass-services with the bridge connected: fixes arrive at the requested interval, and the phone's location indicator is on.
- [ ] Stop (interval 0) and disconnect: the phone's location indicator goes off.
- [ ] Location off on the phone: Glass gets `disabled`. Location back on: fixes resume within 60 s.
- [ ] Permission not granted: Glass gets `permission`.
- [ ] App in the background after boot on Android 14+: note whether fixes flow.
