package org.openmyglass.companion

import org.junit.Assert.assertEquals
import org.junit.Test
import org.openmyglass.corebindings.CoreEvent
import org.openmyglass.corebindings.InternalException

class PollerTest {
    @Test
    fun aPanicInTheCoreIsReportedOnceAndPollingSlowsDownThenRecovers() {
        val answers = ArrayDeque<() -> List<CoreEvent>>()
        repeat(3) { answers.add { throw InternalException("panicked at lib.rs") } }
        answers.add { listOf(CoreEvent.NotesChanged) }
        val delivered = mutableListOf<List<CoreEvent>>()
        val reports = mutableListOf<String>()
        val waits = mutableListOf<Long>()
        val poller = Poller(
            poll = { answers.removeFirst()() },
            deliver = { delivered.add(it) },
            report = { reports.add(it) },
            sleep = { waits.add(it) },
        )
        repeat(4) { poller.step() }

        assertEquals(1, reports.size)
        assertEquals(listOf(500L, 1000L, 2000L, 4000L), waits)
        assertEquals(listOf(listOf(CoreEvent.NotesChanged)), delivered)
        assertEquals(Poller.POLL_INTERVAL_MS, poller.waitMs)
    }

    @Test
    fun theWaitStopsGrowingAtItsLimit() {
        val poller = Poller(
            poll = { throw IllegalStateException("broken") },
            deliver = {},
            report = {},
            sleep = {},
        )
        repeat(20) { poller.step() }
        assertEquals(Poller.MAX_POLL_WAIT_MS, poller.waitMs)
    }
}
