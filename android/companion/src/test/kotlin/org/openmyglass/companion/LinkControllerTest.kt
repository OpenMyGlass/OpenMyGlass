package org.openmyglass.companion

import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.openmyglass.corebindings.CompanionCoreInterface
import org.openmyglass.corebindings.CoreEvent
import org.openmyglass.corebindings.CoreOutput
import org.openmyglass.corebindings.CoreSnapshot
import org.openmyglass.corebindings.GlassState
import org.openmyglass.corebindings.LinkPhase
import org.openmyglass.corebindings.LinkProfile
import org.openmyglass.corebindings.ReconnectBackoffInterface
import java.io.IOException
import java.lang.reflect.Proxy
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.CountDownLatch
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

/** The threads for real, with a fake core and links. */
class LinkControllerTest {
    /** Anything the controller has no business calling fails the test. */
    private val unexpected = Proxy.newProxyInstance(
        CompanionCoreInterface::class.java.classLoader,
        arrayOf(CompanionCoreInterface::class.java),
    ) { _, method, _ -> throw AssertionError("unexpected call to ${method.name}") } as CompanionCoreInterface

    private inner class FakeCore : CompanionCoreInterface by unexpected {
        val linksUp = AtomicInteger()
        val linksDown = AtomicInteger()
        /** What the next pumps return, each whole, so a pump never sees half of a step. */
        val outputs = LinkedBlockingQueue<CoreOutput>()

        fun queue(
            bytes: ByteArray = ByteArray(0),
            closeLink: Boolean = false,
            events: List<CoreEvent> = emptyList(),
        ) {
            outputs.add(CoreOutput(bytes, events, closeLink))
        }

        override fun setFreeBytes(freeBytes: ULong?) = Unit

        override fun linkUp(profile: LinkProfile, allowPairing: Boolean) {
            linksUp.incrementAndGet()
        }

        override fun onBytes(data: ByteArray) = Unit

        override fun linkDown() {
            linksDown.incrementAndGet()
        }

        override fun snapshot() = SNAPSHOT

        override fun pump(): CoreOutput = outputs.poll() ?: CoreOutput(ByteArray(0), emptyList(), false)
    }

    private class FakeBackoff(val waitMs: UInt) : ReconnectBackoffInterface {
        val lasted = ConcurrentLinkedQueue<ULong>()
        val resets = AtomicInteger()

        override fun nextWaitMs(lastedMs: ULong): UInt {
            lasted.add(lastedMs)
            return waitMs
        }

        override fun reset() {
            resets.incrementAndGet()
        }
    }

    /** With [flushHolds], a flush waits for [letFlushEnd], or for the link to close. */
    private class FakeLink(val flushHolds: Boolean = false) : ByteLink {
        val flushing = CountDownLatch(1)
        val letFlushEnd = CountDownLatch(1)
        val calls = ConcurrentLinkedQueue<String>()
        val written = ConcurrentLinkedQueue<ByteArray>()
        val closed = CountDownLatch(1)
        private val end = CountDownLatch(1)

        override val profile = LinkProfile(1024u, 1u, null)

        override fun read(buffer: ByteArray): Int {
            end.await()
            return -1
        }

        override fun write(bytes: ByteArray) {
            calls.add("write")
            written.add(bytes)
        }

        override fun flush() {
            calls.add("flush")
            flushing.countDown()
            if (flushHolds) {
                while (letFlushEnd.count > 0 && closed.count > 0) Thread.sleep(1)
            }
        }

        override fun close() {
            calls.add("close")
            closed.countDown()
            end.countDown()
        }

        /** The other end went away, with nothing said. */
        fun drop() = end.countDown()
    }

    private val core = FakeCore()
    private val failures = LinkedBlockingQueue<String>()
    private val deliveries = AtomicInteger()
    private val delivered = LinkedBlockingQueue<Pair<List<CoreEvent>, ByteLink?>>()
    private var controller: LinkController? = null

    private fun controller(backoff: FakeBackoff, flushLimitMs: Long = 5_000) = LinkController(
        core,
        { 0L },
        backoff,
        { _, why -> failures.add(why) },
        flushLimitMs,
    ) { events, from ->
        delivered.add(events to from)
        deliveries.incrementAndGet()
    }.also { controller = it }

    @After
    fun stop() {
        controller?.disconnect()
    }

    private fun eventually(what: String, check: () -> Boolean) {
        val end = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
        while (!check()) {
            assertTrue("timed out waiting: $what", System.nanoTime() < end)
            Thread.sleep(5)
        }
    }

    /** Long enough that a retry which should not happen would have. */
    private fun assertStays(what: String, check: () -> Boolean) {
        val end = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(400)
        while (System.nanoTime() < end) {
            assertTrue(what, check())
            Thread.sleep(5)
        }
    }

    @Test
    fun aCloseFromTheCoreStillWaitsOutTheBackoff() {
        val c = controller(FakeBackoff(60_000u))
        val opens = AtomicInteger()
        c.connect {
            opens.incrementAndGet()
            FakeLink()
        }
        eventually("the link is up") { core.linksUp.get() == 1 }
        // Closing the link makes the reader wake everyone, which used to end the wait as well.
        core.queue(closeLink = true)
        c.poke()
        eventually("the link is down") { core.linksDown.get() == 1 }
        assertStays("no second attempt during the wait") { opens.get() == 1 }
    }

    @Test
    fun aPokeDoesNotEndTheBackoffButTheOwnerAskingDoes() {
        val backoff = FakeBackoff(60_000u)
        val c = controller(backoff)
        val opens = AtomicInteger()
        c.connect {
            opens.incrementAndGet()
            throw IOException("nothing there")
        }
        eventually("the wait began") { backoff.lasted.size == 1 }
        c.poke()
        assertStays("a poke is not a reason to scan") { opens.get() == 1 }

        val link = FakeLink()
        c.connect { link }
        eventually("the new link runs at once") { core.linksUp.get() == 1 }
        assertEquals(1, backoff.resets.get())
    }

    @Test
    fun aFailureOfAnyKindIsReportedAndRetried() {
        val c = controller(FakeBackoff(1u))
        val opens = AtomicInteger()
        c.connect {
            when (opens.incrementAndGet()) {
                1 -> throw IOException("no Glass found")
                2 -> throw IllegalStateException("BT Adapter is not turned ON")
                else -> FakeLink()
            }
        }
        eventually("it came through both") { core.linksUp.get() == 1 }
        assertEquals("no Glass found", failures.poll())
        assertEquals("BT Adapter is not turned ON", failures.poll())
    }

    @Test
    fun anAttemptNobodyWantsAnyMoreIsCancelledAndNotReported() {
        val c = controller(FakeBackoff(1u))
        val started = CountDownLatch(1)
        c.connect { cancelled ->
            started.countDown()
            while (!cancelled()) Thread.sleep(1)
            throw IOException("cancelled")
        }
        assertTrue(started.await(5, TimeUnit.SECONDS))
        c.disconnect()
        eventually("the attempt ended") { deliveries.get() >= 1 }
        assertTrue(failures.isEmpty())
    }

    @Test
    fun aLinkFromAReplacedOpenerIsClosedAndNeverRun() {
        val c = controller(FakeBackoff(60_000u))
        val inOldOpener = CountDownLatch(1)
        val letItFinish = CountDownLatch(1)
        val old = FakeLink()
        c.connect {
            inOldOpener.countDown()
            letItFinish.await()
            old
        }
        assertTrue(inOldOpener.await(5, TimeUnit.SECONDS))
        val new = FakeLink()
        c.connect { new }
        letItFinish.countDown()
        assertTrue("the old one closes", old.closed.await(5, TimeUnit.SECONDS))
        eventually("the new one runs") { c.running === new && core.linksUp.get() == 1 }
        assertEquals(1, core.linksUp.get())
    }

    @Test
    fun askingAgainWithTheSameOpenerIsStillANewRequest() {
        val c = controller(FakeBackoff(60_000u))
        val links = LinkedBlockingQueue<FakeLink>()
        val opener: LinkOpener = { FakeLink().also { links.add(it) } }
        val first = c.connect(opener)
        eventually("the first is up") { core.linksUp.get() == 1 }
        val second = c.connect(opener)
        assertTrue(first != second)
        assertFalse(c.wants(first))
        assertTrue(c.wants(second))
        // At once, not after the 60 s wait a retry of the same request would get.
        eventually("the second is up") { core.linksUp.get() == 2 }
        assertTrue(links.take().closed.await(5, TimeUnit.SECONDS))
    }

    @Test
    fun aRequestIsNoLongerWantedOnceStopped() {
        val c = controller(FakeBackoff(60_000u))
        val request = c.connect { throw IOException("no Glass found") }
        eventually("it failed") { failures.size == 1 }
        assertTrue(c.wants(request))
        c.disconnect()
        assertFalse(c.wants(request))
    }

    @Test
    fun theLastFrameIsFlushedBeforeACloseTheCoreAskedFor() {
        val c = controller(FakeBackoff(60_000u))
        val link = FakeLink()
        c.connect { link }
        eventually("the link is up") { core.linksUp.get() == 1 }
        val last = byteArrayOf(7)
        core.queue(last, closeLink = true)
        c.poke()
        eventually("the link is down") { core.linksDown.get() == 1 }
        assertEquals(listOf("write", "flush", "close"), link.calls.toList())
        assertArrayEquals(last, link.written.single())
    }

    @Test
    fun stoppingAfterAFlushWritesWhatIsQueuedThenStaysDown() {
        val c = controller(FakeBackoff(1u))
        val opens = AtomicInteger()
        val link = FakeLink()
        c.connect {
            opens.incrementAndGet()
            link
        }
        eventually("the link is up") { core.linksUp.get() == 1 }
        core.queue(byteArrayOf(9))
        c.disconnectAfterFlush()
        eventually("the link is down") { core.linksDown.get() == 1 }
        assertEquals(listOf("write", "flush", "close"), link.calls.toList())
        assertStays("it does not come back") { opens.get() == 1 }
    }

    @Test
    fun aConnectRightAfterUnpairWaitsForTheFlush() {
        val c = controller(FakeBackoff(60_000u))
        val old = FakeLink(flushHolds = true)
        c.connect { old }
        eventually("the link is up") { core.linksUp.get() == 1 }
        core.queue(byteArrayOf(5), closeLink = true)
        c.disconnectAfterFlush()
        assertTrue(old.flushing.await(5, TimeUnit.SECONDS))
        // The owner taps Connect while the Unpair is on its way.
        val new = FakeLink()
        c.connect { new }
        assertStays("the flush is not cut short") { old.closed.count == 1L }
        assertEquals(1, core.linksUp.get())
        old.letFlushEnd.countDown()
        eventually("then the new link runs") { core.linksUp.get() == 2 && c.running === new }
        assertEquals(listOf("write", "flush", "close"), old.calls.toList())
    }

    @Test
    fun aFlushThatHangsIsCutAtTheLimit() {
        val c = controller(FakeBackoff(60_000u), flushLimitMs = 600)
        val old = FakeLink(flushHolds = true)
        c.connect { old }
        eventually("the link is up") { core.linksUp.get() == 1 }
        c.disconnectAfterFlush()
        assertTrue(old.flushing.await(5, TimeUnit.SECONDS))
        val new = FakeLink()
        c.connect { new }
        assertTrue("closed at the limit", old.closed.await(5, TimeUnit.SECONDS))
        eventually("then the new link runs") { core.linksUp.get() == 2 }
    }

    @Test
    fun eventsComeWithTheLinkTheyCameFrom() {
        val c = controller(FakeBackoff(60_000u))
        val link = FakeLink()
        c.connect { link }
        eventually("the link is up") { core.linksUp.get() == 1 }
        val running = CoreEvent.LinkRunning("Glass", true)
        core.queue(events = listOf(running))
        c.poke()
        val (events, from) = delivered.poll(5, TimeUnit.SECONDS)!!
        assertEquals(listOf(running), events)
        assertTrue(from === link)
        link.drop()
        eventually("the end is delivered with no link") { delivered.any { it.second == null } }
    }

    @Test
    fun aLinkThatJustDroppedIsStillNoticedByTheScreen() {
        val c = controller(FakeBackoff(60_000u))
        val link = FakeLink()
        c.connect { link }
        eventually("the link is up") { core.linksUp.get() == 1 }
        assertEquals(0, deliveries.get())
        link.drop()
        eventually("the screen hears, with no event to carry it") { deliveries.get() == 1 }
        assertFalse(c.running === link)
    }

    @Test
    fun itStartsAgainAfterStayingDown() {
        val backoff = FakeBackoff(1u)
        val c = controller(backoff)
        val first = FakeLink()
        c.connect { first }
        eventually("the first is up") { core.linksUp.get() == 1 }
        c.disconnect()
        eventually("the first is down") { core.linksDown.get() == 1 }
        // Right after, whichever side of the supervisor's exit this lands on.
        val second = FakeLink()
        c.connect { second }
        eventually("the second is up") { core.linksUp.get() == 2 }
    }

    @Test
    fun theBackoffHearsHowLongTheLinkLasted() {
        val backoff = FakeBackoff(60_000u)
        val c = controller(backoff)
        val link = FakeLink()
        c.connect { link }
        eventually("the link is up") { core.linksUp.get() == 1 }
        Thread.sleep(50)
        link.drop()
        eventually("the wait began") { backoff.lasted.size == 1 }
        assertTrue(backoff.lasted.single() >= 50u)
    }

    private companion object {
        val NOTHING_KNOWN = GlassState(null, null, null, null, null, null, null, null, null, null, null)
        val SNAPSHOT = CoreSnapshot(LinkPhase.DOWN, "Glass", "key", NOTHING_KNOWN, null, 0u)
    }
}
