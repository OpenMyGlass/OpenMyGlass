# protocol

Message definitions shared by all OpenMyGlass clients.

| Path | Content |
|---|---|
| `companion/companion.proto` | Subset of the original Glass companion protocol (`Envelope` and the messages we send or parse), copied from the reconstructed XE24 schema in [`docs/protocol/proto/glass-xe24/`](../docs/protocol/proto/glass-xe24/). See [`docs/protocol/`](../docs/protocol/) for transport, framing and sequences. |
| `openmyglass/v1/` | Our own bridge protocol (added with the bridge app). |

The Gradle module `:protocol` generates Java lite classes in `org.openmyglass.protocol.companion` (outer class `Companion`).

```bash
./gradlew :protocol:test
```
