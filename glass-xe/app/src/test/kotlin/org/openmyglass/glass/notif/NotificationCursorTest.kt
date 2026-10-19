package org.openmyglass.glass.notif

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class NotificationCursorTest {
    @Test
    fun `the selected card stays the same notification when others arrive or leave`() {
        // Review finding: an index-based selection made Dismiss act on another notification
        val c = NotificationCursor()
        assertEquals(0, c.index(listOf("a", "b", "c")))
        c.next(listOf("a", "b", "c"))
        assertEquals("b", c.key())
        assertEquals(2, c.index(listOf("new", "a", "b", "c")))   // a new one on top: still on b
        assertEquals(0, c.index(listOf("b", "c")))               // a removed: still on b
    }

    @Test
    fun `a removed selection falls back to the same position`() {
        val c = NotificationCursor()
        c.index(listOf("a", "b", "c")); c.next(listOf("a", "b", "c"))
        assertEquals(1, c.index(listOf("a", "c")))
        assertEquals("c", c.key())
    }

    @Test
    fun `previous stops at the first card and empty lists have no selection`() {
        val c = NotificationCursor()
        c.index(listOf("a", "b")); c.previous(listOf("a", "b"))
        assertEquals("a", c.key())
        assertEquals(-1, c.index(emptyList()))
        assertNull(c.key())
    }
}
