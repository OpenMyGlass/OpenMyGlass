# protocol

Message definitions shared by all OpenMyGlass clients.

| Path | Content |
|---|---|
| `companion/companion.proto` | Subset of the original Glass companion protocol (`Envelope` and the messages we send or parse), copied from the reconstructed XE24 schema in [`docs/protocol/proto/glass-xe24/`](../docs/protocol/proto/glass-xe24/). See [`docs/protocol/`](../docs/protocol/) for transport, framing and sequences. |
| `openmyglass/v1/bridge.proto` | Our own bridge protocol between the phone and the OpenMyGlass bridge app on Glass (navigation state, icons). |

The Gradle module `:protocol` generates Java lite classes in `org.openmyglass.protocol.companion` (outer class `Companion`) and `org.openmyglass.protocol.v1` (outer class `Bridge`).

```bash
./gradlew :protocol:test
```
