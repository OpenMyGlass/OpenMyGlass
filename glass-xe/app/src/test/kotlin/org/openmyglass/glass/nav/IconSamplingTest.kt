package org.openmyglass.glass.nav

import org.junit.Assert.assertEquals
import org.junit.Test

class IconSamplingTest {
    @Test
    fun `sample size keeps decoded icons small`() {
        assertEquals(1, CardBinder.sampleSizeFor(126, 126, 160))
        assertEquals(1, CardBinder.sampleSizeFor(160, 160, 160))
        assertEquals(2, CardBinder.sampleSizeFor(320, 320, 160))
        assertEquals(16, CardBinder.sampleSizeFor(3000, 3000, 160))
        assertEquals(0, CardBinder.sampleSizeFor(0, 100, 160))   // unreadable bounds: do not decode
    }
}
