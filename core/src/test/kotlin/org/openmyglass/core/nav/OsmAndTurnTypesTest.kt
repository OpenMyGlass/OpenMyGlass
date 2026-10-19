package org.openmyglass.core.nav

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class OsmAndTurnTypesTest {

    @Test
    fun `known turn types map to maneuvers`() {
        val expected = mapOf(
            1 to Maneuver.STRAIGHT, 2 to Maneuver.LEFT, 3 to Maneuver.SLIGHT_LEFT, 4 to Maneuver.SHARP_LEFT,
            5 to Maneuver.RIGHT, 6 to Maneuver.SLIGHT_RIGHT, 7 to Maneuver.SHARP_RIGHT, 8 to Maneuver.KEEP_LEFT,
            9 to Maneuver.KEEP_RIGHT, 10 to Maneuver.U_TURN_LEFT, 11 to Maneuver.U_TURN_RIGHT, 12 to Maneuver.OFF_ROUTE,
            13 to Maneuver.ROUNDABOUT, 14 to Maneuver.ROUNDABOUT_LEFT,
        )
        expected.forEach { (type, maneuver) -> assertEquals(maneuver, OsmAndTurnTypes.toManeuver(type), "turnType $type") }
    }

    @Test
    fun `unknown turn types map to UNKNOWN`() {
        listOf(0, 15, -1, 999).forEach { assertEquals(Maneuver.UNKNOWN, OsmAndTurnTypes.toManeuver(it), "turnType $it") }
    }
}
