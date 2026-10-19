# sim-glass

A simulated Glass XE24 companion server over TCP, for testing OpenMyGlass without hardware.
It mirrors what the firmware does according to [`docs/protocol/`](../../docs/protocol/):

- sends its handshake first (`version` 6.10, `androidDeviceInfo{sdkVersion=19}`, `glassFeatureInfoG2C`);
- reads the companion's handshake and warns when the companion's major version is newer (real Glass would ask for an OTA update);
- answers `glassInfoRequestC2G` with configurable device info (battery, storage, name, software version, 640×360 screen);
- records `timezoneC2G`;
- accepts **one client at a time** and closes any second connection immediately.

It does not emulate the firmware itself: passing against sim-glass means we follow our reconstructed spec, not that real Glass accepts us. See the hardware checklist in [`docs/progress.md`](../../docs/progress.md).

## Run

```bash
./gradlew :tools:sim-glass:run --args="--port 7823"        # default port 7823
./gradlew :tools:sim-glass:run --args="--port 7823 --battery 55"
./gradlew :tools:sim-glass:run --args="--silent"            # never sends the handshake
```

An Android emulator reaches it with `adb reverse tcp:7823 tcp:7823`.

## Tests

```bash
./gradlew :tools:sim-glass:test   # end-to-end: core ConnectionManager over TCP against SimGlass
```
