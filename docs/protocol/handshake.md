# Handshake and authentication

Main source: XE24 `GlassBluetooth` → `bluetooth/companion/CompanionConnection.handshake()`, `companion/CompanionMessagingUtil`, `companion/CompanionConstants`, `bluetooth/companion/CompanionSocketServer`.

## Protocol version

- `version = (minor << 16) + major` (`CompanionConstants.calculateVersion`).
- Glass XE24: major **6**, minor **10**, so `version = 655366`.
- MyGlass 3.5.8: major **6**, minor **11**, so `version = 720902`. 3.5.8 adds `SERVICE_PING_MIN_MINOR_VERSION = 11`.
- Several features are gated on the other side's minor version (`*_MIN_MINOR_VERSION`): screencast with screen off ≥3, Wi-Fi password typed on the phone ≥4, silent credential update ≥5, photo sync ≥6, photo delete ≥7, number of photos to sync ≥8, photo sync settings ≥9, notification sync ≥10.

## Sequence

```
Phone (client)                              Glass (server, "Companion" F15CC914-…)
     |  RFCOMM secure connect  ------------------->  accept()
     |                                               prepareConnection() (single connection)
     |  <-------------------  Envelope #1 (G2C)
     |       version=655366, timeMillis, uptimeMillis,
     |       androidDeviceInfo{sdkVersion=19},
     |       glassFeatureInfoG2C{isWallpaperFeatureEnabled}
     |  Envelope #1 (C2G)  ------------------------->
     |       version, [timezoneC2G], [companionDeviceInfoC2G],
     |       [androidDeviceInfo{sdkVersion}], [companionFeatureInfoC2G]
     |                                               setRemoteVersion(); if different → notifyVersionMismatch()
     |                                               updates time zone, phone info, features
     |  <======== free exchange of Envelopes (see messages.md) ========>
```

- **Glass speaks first.** The phone must read one Envelope and then answer with its own.
- Every Envelope carries `version`, `timeMillis` and `uptimeMillis` (`CompanionMessagingUtil.newEnvelope`).
- If the handshake does not arrive or cannot be parsed, Glass throws `IOException("Failed to parse handshake envelope.")` and closes.

## Authentication

- **No application-level authentication.** The handshake has no tokens, signatures, challenges or Google account. The only protection is **Bluetooth bonding**, since the socket is secure.
- **Version mismatch** does not block the connection. If the companion's *major* is higher than Glass's, Glass shows a full-screen message asking for an OTA update (`R.string.version_mismatch_require_glass_ota`); otherwise the connection continues (`CompanionSocketServer.notifyVersionMismatch`).
- **No identity check on the phone app:** Glass verifies neither the package nor the signature of the client. Any app on the bonded phone can connect to the `Companion` service.
- `UpdateCredentialRequest` (field 29, G2C) carries `accountName` and `accountType`: Glass can ask the phone to refresh an account credential. This concerns cloud services, not the link. The phone uses `companion/auth/CompanionAuthUtils` (AccountManager) to get OAuth tokens for Google servers.

## Google account on XE24

- `GlassLiteSetup` (`litesetup/SetupActivity`, XE24) sets `Settings.Global.device_provisioned = 1` on launch, activates the device admin, renames the Bluetooth device and **disables itself**. First-run setup asks for no account and no connection.
- On XE24 no Glass code reads `glassSetupRequestC2G` (field 25). MyGlass 3.5.8 still sends it (`com/google/android/apps/myglass/service/CompanionService`), but the firmware ignores it.
- Conclusion: on XE24 **the companion channel works without a Google account** on either side, as far as the code shows. **[to verify on hardware]**

## Consequences for option A (emulating MyGlass)

Feasible at protocol level: a third-party app can connect, complete the handshake with `version` 6.10 and speak the protocol with no secrets. The limits are functional — which messages Glass actually handles and which features depend on the cloud: see `messages.md` and `../cloud-dependencies.md`.
