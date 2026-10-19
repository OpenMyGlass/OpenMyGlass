package org.openmyglass.core.nav

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Test

class NavUpdateReducerTest {

    private val reducer = NavUpdateReducer("osmand")

    @Test
    fun `direction produces an update`() {
        assertEquals(NavUpdate(Maneuver.LEFT, 250, null, null, null, "osmand"), reducer.onDirection(250, 2))
    }

    @Test
    fun `voice text becomes the instruction`() {
        reducer.onDirection(250, 2)
        assertEquals("Turn left onto Via Roma", reducer.onVoicePlayed(listOf("", "Turn left onto Via Roma"))?.instruction)
        assertEquals("Turn left onto Via Roma", reducer.onDirection(120, 2).instruction)
    }

    @Test
    fun `voice before any direction yields nothing`() {
        assertNull(reducer.onVoicePlayed(listOf("x")))
    }

    @Test
    fun `negative distance is unknown`() {
        assertNull(reducer.onDirection(-1, 5).distanceMeters)
    }

    @Test
    fun `stop clears state`() {
        reducer.onDirection(250, 2)
        reducer.onVoicePlayed(listOf("Turn left"))
        assertNull(reducer.onNavigationStopped())
        assertNull(reducer.current)
        assertNull(reducer.onVoicePlayed(listOf("Turn right")))
    }
}
