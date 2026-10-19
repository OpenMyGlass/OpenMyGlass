package org.openmyglass.protocol

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test
import org.openmyglass.protocol.companion.Companion.Envelope
import org.openmyglass.protocol.companion.Companion.GlassInfoResponse

class CompanionProtoTest {

    @Test
    fun `envelope round-trips version and glass info`() {
        val original = Envelope.newBuilder()
            .setVersion(655366)
            .setTimeMillis(1)
            .setGlassInfoResponseG2C(
                GlassInfoResponse.newBuilder().setBatteryLevel(80).setDeviceName("Glass")
            )
            .build()

        val parsed = Envelope.parseFrom(original.toByteArray())

        assertEquals(655366, parsed.version)
        assertEquals(1L, parsed.timeMillis)
        assertEquals(80, parsed.glassInfoResponseG2C.batteryLevel)
        assertEquals("Glass", parsed.glassInfoResponseG2C.deviceName)
    }

    @Test
    fun `unknown fields are ignored`() {
        val known = Envelope.newBuilder().setVersion(655366).build().toByteArray()
        // field 45 (tag (45 << 3) | 2 = 0xEA 0x02, 3 bytes) and field 47 (tag 0xFA 0x02, 1 byte):
        // fields used by newer MyGlass/firmware versions that this subset does not model
        val unknown = byteArrayOf(0xEA.toByte(), 0x02, 0x03, 1, 2, 3, 0xFA.toByte(), 0x02, 0x01, 9)

        val parsed = Envelope.parseFrom(known + unknown)

        assertEquals(655366, parsed.version)
    }
}
