# Companion protocol transport

Sources: `XE24` = `_re/jadx/GlassBluetooth` (the same classes ship in every Glass app), `MG` = `_re/jadx/phone-myglass-3.5.8`.
Each statement cites the class it comes from. Glass class names are not obfuscated; phone ones are (e.g. `defpackage/tr`).

## Bluetooth channels

Everything runs over **Bluetooth Classic RFCOMM**. The brief listed this as an assumption; the code confirms it.

| Channel | Listener | SDP name | UUID | Security | Source |
|---|---|---|---|---|---|
| **Companion** (main channel) | Glass (server) | `Companion` | `F15CC914-E4BC-45CE-9930-CB7695385850` | secure (bonding required) | XE24 `bluetooth/companion/BluetoothSocketThread` → `listenUsingRfcommWithServiceRecord(SOCKET_NAME, SECURE_UUID)`; MG `defpackage/tr` → `createRfcommSocketToServiceRecord(SECURE_UUID)` |
| **Ping** | phone (server) | `PingCompanion` | `DC935393-831A-4736-89E3-C8CE5BB20EEA` | insecure | MG `defpackage/tq` → `listenUsingInsecureRfcommWithServiceRecord(PING_SOCKET_NAME, PING_SOCKET_UUID)` |
| **Proxy** (phone Internet for Glass) | phone (server) | `Glass Proxy` | `fafbdd20-83f0-4389-addf-917ac9dae5b1` | secure | MG `defpackage/ry` → `listenUsingRfcommWithServiceRecord(PROXY_SDP_NAME, PROXY_UUID)` |
| **Identity** (lets a phone recognize a Glass) | Glass (server) | constant `IDENTITY_SDP_NAME` | `f96647cf-7f25-4277-843d-f407b4192f8b` | insecure | XE24 `bluetooth/identity/GlassIdentityProfile` |

The standard UUIDs found in the same classes are `0x1108` (Headset), `0x1116` (NAP) and `0x111E`/`0x111F` (Hands-Free). Glass uses them for audio and tethering through standard profiles (`bluetooth/handsfree/*`, `bluetooth/tethering/*`), not for the companion protocol.

**Roles:** Glass is the server and the phone is the client. A companion on Android therefore only needs to open an RFCOMM socket to the already-bonded Glass; it does not have to expose a service of its own for the main channel.

## Framing

Each message is a protobuf `Envelope` (proto2, *nano* runtime) prefixed with its length as a **varint32**. This is the same format as `writeDelimitedTo`/`parseDelimitedFrom` in the standard protobuf library.

- Source: XE24 `protobuf/DelimitedProtoIo` (`writeRawVarint32(size)` followed by the message bytes; on read, `readRawVarint32` and then a full read of the buffer).
- End of stream: if `read()` returns -1 on the first byte, `parseDelimitedFrom` returns `null` and the connection is closed (`CompanionConnection.run` throws `IOException("Could not parse Envelope")`).
- A single full-duplex stream. On the way out, Glass serializes sends through `EnvelopeSender` (asynchronous, with `sendOrUpdateEnvelopeAsync(id, …)` to replace a message that has not been sent yet).

## Pairing

- The companion channel is *secure*, so it requires standard Bluetooth bonding. There is no extra application-level pairing.
- `BluetoothPairingPolicy` (XE24) holds Glass's pairing acceptance rules. To analyze if Phase 3 needs automated pairing.

## Keep-alive and reconnection

- **No application-level ping** on the companion channel: the `Envelope` has no heartbeat message. A dropped connection is detected by an I/O error on the socket.
- **Glass-side "ping"**: XE24 `bluetooth/companion/PingRequester` runs `fetchUuidsWithSdp()` against the bonded phone (`companion/UuidFetcher`). It does so immediately, after 30 s (`getPingShortInterval`) or every 300 s (`getPingDefaultInterval`), and only while the device is bonded (state 12). During the ping it temporarily turns off BT/Wi-Fi coexistence mode (`HiddenWifiManager.setBtcCtsMode(false)`).
- **Phone side**: MG `defpackage/tq` accepts connections on the `PingCompanion` socket, closes them right away and sends the broadcast `com.google.glass.companion.GLASS_PING`, which makes the companion reconnect.
  **[unverified]** On XE24 the ping is only an SDP query and never opens `PingCompanion`; it is still unclear which event makes MyGlass 3.5.8 reconnect in that case (for example `ACL_CONNECTED`).
- XE24 `bluetooth/companion/ConnectionRestorer` remembers the last connected device and schedules pings (`AT_ONCE` on disconnect, `SHORT` on retries).
- The Glass server accepts **one connection at a time** (`CompanionSocketServer.prepareConnection`; log "always a connection running").

## Implications for OpenMyGlass

1. A third-party Android app can open a secure RFCOMM socket to Glass with UUID `F15CC914-…`, using public APIs (`createRfcommSocketToServiceRecord`).
2. The companion has to handle reconnection itself: retry when Glass is reachable, and expose the `PingCompanion` socket for compatibility.
3. The phone acting as Internet proxy (`Glass Proxy`) is a separate feature for Phase 3. The proxy traffic format has not been analyzed yet **[unverified]**.
