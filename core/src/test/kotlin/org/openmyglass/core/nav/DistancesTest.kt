package org.openmyglass.core.nav

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Test

class DistancesTest {
    @Test
    fun `parses metric and imperial in both decimal styles`() {
        assertEquals(200, Distances.toMeters("200 m"))
        assertEquals(2800, Distances.toMeters("2,8 km"))
        assertEquals(1300, Distances.toMeters("1.3 km"))
        assertEquals(213, Distances.toMeters("700 ft"))
        assertEquals(483, Distances.toMeters("0.3 mi"))
        assertEquals(91, Distances.toMeters("100 yd"))
    }

    @Test
    fun `returns null without a number and a unit`() {
        assertNull(Distances.toMeters("verso Äulestrasse"))
        assertNull(Distances.toMeters(""))
        assertNull(Distances.toMeters("Avvio della navigazione in corso…"))
    }
}
