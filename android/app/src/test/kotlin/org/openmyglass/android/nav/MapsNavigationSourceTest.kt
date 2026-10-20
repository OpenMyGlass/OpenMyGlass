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

    @Test
    fun `an empty update keeps the last state`() {
        // Issue #35: Maps briefly posts the notification with no text; that is not the end of navigation
        val source = MapsNavigationSource()
        source.onPosted(NotificationText("200 m", "28 / Landstrasse", "x · y · 8:33 PM ETA", null, null))
        source.onPosted(NotificationText(null, null, null, null, null))
        assertEquals(200, source.current.value?.distanceMeters)
    }
}
