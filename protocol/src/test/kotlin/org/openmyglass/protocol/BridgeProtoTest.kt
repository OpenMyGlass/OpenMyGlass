package org.openmyglass.protocol

import com.google.protobuf.ByteString
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Test
import org.openmyglass.protocol.v1.Bridge.Frame
import org.openmyglass.protocol.v1.Bridge.Maneuver
import org.openmyglass.protocol.v1.Bridge.NavState

class BridgeProtoTest {

    @Test
    fun `frame round-trips nav state`() {
        val sha = ByteString.copyFrom(ByteArray(32) { it.toByte() })
        val nav = NavState.newBuilder()
            .setManeuver(Maneuver.ROUNDABOUT).setDistanceM(143).setInstruction("Take the 2nd exit")
            .setStreet("Landstrasse").setEta("8:33 PM").setSource("osmand").setIconSha256(sha)
        val parsed = Frame.parseFrom(Frame.newBuilder().setNav(nav).build().toByteArray())
        assertEquals(Frame.BodyCase.NAV, parsed.bodyCase)
        assertEquals(nav.build(), parsed.nav)
    }

    @Test
    fun `absent optional fields stay absent`() {
        val parsed = NavState.parseFrom(NavState.newBuilder().setSource("gmaps").build().toByteArray())
        assertFalse(parsed.hasDistanceM())
        assertFalse(parsed.hasStreet())
        assertFalse(parsed.hasIconSha256())
    }

    @Test
    fun `maneuver values match core`() {
        assertEquals(13, Maneuver.ROUNDABOUT.number)
        assertEquals(15, Maneuver.ARRIVE.number)
        assertEquals(0, Maneuver.MANEUVER_UNKNOWN.number)
    }
}
