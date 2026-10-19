package org.openmyglass.android.nav

import org.junit.Assert.assertEquals
import org.junit.Test

class IconEncoderTest {
    @Test
    fun `scale keeps aspect and caps at 128`() {
        assertEquals(128 to 64, IconEncoder.scaleTo(500, 250))
        assertEquals(64 to 64, IconEncoder.scaleTo(64, 64))
        assertEquals(128 to 128, IconEncoder.scaleTo(126 * 4, 126 * 4))
        assertEquals(1 to 128, IconEncoder.scaleTo(1, 2000))
    }
}
