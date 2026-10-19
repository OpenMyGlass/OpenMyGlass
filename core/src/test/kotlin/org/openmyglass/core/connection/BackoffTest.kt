package org.openmyglass.core.connection

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test
import kotlin.time.Duration.Companion.seconds

class BackoffTest {
    @Test
    fun `backoff doubles and caps`() {
        val b = Backoff()
        assertEquals(listOf(1, 2, 4, 8, 16, 32, 60, 60).map { it.seconds }, List(8) { b.next() })
        b.reset()
        assertEquals(1.seconds, b.next())
    }
}
