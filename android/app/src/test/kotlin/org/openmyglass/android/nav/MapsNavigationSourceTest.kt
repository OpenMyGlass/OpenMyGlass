package org.openmyglass.android.nav

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.openmyglass.core.nav.NotificationText

class MapsNavigationSourceTest {
    @Test
    fun `posted update becomes current`() {
        val source = MapsNavigationSource()
        source.onPosted(NotificationText("200 m", "28 / Landstrasse", "2 min · 1.3 km · 8:33 PM ETA", null, null))
        assertEquals(200, source.current.value?.distanceMeters)
        assertEquals("gmaps", source.name)
    }

    @Test
    fun `removal emits null`() {
        val source = MapsNavigationSource()
        source.onPosted(NotificationText("200 m", "28", "x · y · 8:33 PM ETA", null, null))
        source.onRemoved()
        assertNull(source.current.value)
    }
}
