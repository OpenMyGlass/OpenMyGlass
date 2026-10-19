package org.openmyglass.glass.nav

import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class IconCacheTest {
    @Test
    fun `cache evicts the least recently used`() {
        val cache = IconCache(2)
        cache.put("a", byteArrayOf(1)); cache.put("b", byteArrayOf(2))
        cache.get("a")
        cache.put("c", byteArrayOf(3))
        assertNotNull(cache.get("a"))
        assertNull(cache.get("b"))
        assertNotNull(cache.get("c"))
    }

    @Test
    fun `an icon is requested only once until it arrives`() {
        val cache = IconCache(2)
        assertTrue(cache.markRequested("a"))
        assertFalse(cache.markRequested("a"))
        cache.put("a", byteArrayOf(1))
        assertFalse(cache.markRequested("a"))
    }
}
