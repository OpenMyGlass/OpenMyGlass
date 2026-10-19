package org.openmyglass.glass.nav

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.openmyglass.protocol.v1.Bridge

class WakePolicyTest {
    private fun card(maneuver: Bridge.Maneuver, distance: Int?) = NavCardState.Card(
        Bridge.NavState.newBuilder().setManeuver(maneuver).setSource("osmand").also { b -> distance?.let { b.setDistanceM(it) } }.build(),
        null, null,
    )

    @Test fun `wakes on the first card`() = assertTrue(WakePolicy.shouldWake(null, card(Bridge.Maneuver.LEFT, 800)))
    @Test fun `wakes when crossing 200 m`() = assertTrue(WakePolicy.shouldWake(card(Bridge.Maneuver.LEFT, 230), card(Bridge.Maneuver.LEFT, 190)))
    @Test fun `wakes on a new maneuver`() = assertTrue(WakePolicy.shouldWake(card(Bridge.Maneuver.LEFT, 30), card(Bridge.Maneuver.RIGHT, 900)))
    @Test fun `does not wake on small distance changes`() {
        assertFalse(WakePolicy.shouldWake(card(Bridge.Maneuver.LEFT, 900), card(Bridge.Maneuver.LEFT, 850)))
        assertFalse(WakePolicy.shouldWake(card(Bridge.Maneuver.LEFT, 150), card(Bridge.Maneuver.LEFT, 120)))
    }
    @Test fun `does not wake when navigation ends`() = assertFalse(WakePolicy.shouldWake(card(Bridge.Maneuver.LEFT, 150), null))
}
