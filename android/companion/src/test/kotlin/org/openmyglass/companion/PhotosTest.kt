package org.openmyglass.companion

import org.junit.Assert.assertEquals
import org.junit.Test

class PhotosTest {
    @Test
    fun smallPhotosAreLeftAlone() {
        assertEquals(1, Photos.sampleSize(640, 360, 640))
        assertEquals(1, Photos.sampleSize(10, 10, 640))
    }

    @Test
    fun theLongerSideDecides() {
        assertEquals(2, Photos.sampleSize(641, 10, 640))
        assertEquals(2, Photos.sampleSize(10, 641, 640))
        assertEquals(8, Photos.sampleSize(4000, 3000, 640))
    }

    @Test
    fun aSizeOfNothingStillEnds() {
        assertEquals(1, Photos.sampleSize(0, 0, 640))
    }
}
