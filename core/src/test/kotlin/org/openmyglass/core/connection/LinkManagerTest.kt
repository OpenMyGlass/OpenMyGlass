package org.openmyglass.core.connection

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.openmyglass.core.transport.Channel
import org.openmyglass.core.transport.Transport
import org.openmyglass.core.transport.TransportKind
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.util.Collections
import java.util.concurrent.atomic.AtomicInteger
import kotlin.time.Duration
import kotlin.time.Duration.Companion.milliseconds
import kotlin.time.Duration.Companion.seconds

class LinkManagerTest {

    private object DummyChannel : Channel {
        override val input = ByteArrayInputStream(ByteArray(0))
        override val output = ByteArrayOutputStream()
        override fun close() {}
    }

    private class FakeTransport(private val failures: Int = 0) : Transport {
        override val kind = TransportKind.TCP
        val connects = AtomicInteger()
        override fun connect(): Channel {
            if (connects.incrementAndGet() <= failures) throw IOException("refused")
            return DummyChannel
        }
    }

    /** A session that handshakes immediately and runs until [end] completes (normally or with an error). */
    private class FakeSession : LinkSession<String> {
        val end = CompletableDeferred<Unit>()
        var closed = false
        override suspend fun handshake() = "hello"
        override suspend fun run() = end.await()
        override fun close() { closed = true; end.complete(Unit) }
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val history = Collections.synchronizedList(mutableListOf<LinkState<String>>())
    private val sessions = Collections.synchronizedList(mutableListOf<FakeSession>())

    private fun manager(transport: () -> Transport?, initial: Duration = 50.milliseconds, max: Duration = 200.milliseconds,
                        stableAfter: Duration = Duration.ZERO, session: () -> FakeSession = { FakeSession() }) =
        LinkManager(transport, { session().also { sessions += it } }, scope, Backoff(initial, max),
            onStateChange = { history += it }, stableAfter = stableAfter)

    private val cursor = HistoryCursor(history)

    @Suppress("UnusedReceiverParameter")
    private fun LinkManager<String>.await(timeout: Duration = 3.seconds, p: (LinkState<String>) -> Boolean) =
        cursor.await(timeout, p)

    @AfterEach
    fun tearDown() = scope.cancel()

    @Test
    fun `retries with backoff`() {
        val t = FakeTransport(failures = 2)
        val m = manager({ t }); m.start()
        m.await { it is LinkState.Connected }
        assertEquals(listOf(LinkState.WaitingToRetry(1, 50.milliseconds), LinkState.WaitingToRetry(2, 100.milliseconds)),
            history.filterIsInstance<LinkState.WaitingToRetry>())
    }

    @Test
    fun `unavailable transport backs off without connecting`() {
        val m = manager({ null }); m.start()
        m.await { it is LinkState.WaitingToRetry && (it as LinkState.WaitingToRetry).attempt >= 3 }
        assertTrue(history.any { it is LinkState.Unavailable })
        assertTrue(sessions.isEmpty())
    }

    @Test
    fun `reconnectNow skips the wait`() {
        val t = FakeTransport(failures = 1)
        val m = manager({ t }, initial = 10.seconds, max = 10.seconds); m.start()
        m.await { it is LinkState.WaitingToRetry }
        m.reconnectNow()
        m.await(2.seconds) { it is LinkState.Connected }
    }

    @Test
    fun `stop is final`() {
        val t = FakeTransport()
        val m = manager({ t }); m.start()
        m.await { it is LinkState.Connected }
        m.stop()
        m.await { it == LinkState.Stopped }
        Thread.sleep(300)
        assertEquals(1, t.connects.get())
        assertTrue(sessions.single().closed)
    }

    @Test
    fun `a failing session ends and retries`() {
        val t = FakeTransport()
        val m = manager({ t }); m.start()
        m.await { it is LinkState.Connected }
        sessions.first().end.completeExceptionally(IOException("dropped"))
        m.await { it is LinkState.Connected }
        assertTrue(history.any { it is LinkState.WaitingToRetry })
    }

    @Test
    fun `failures are reported`() {
        val errors = Collections.synchronizedList(mutableListOf<Throwable>())
        val m = LinkManager({ FakeTransport(failures = 1) }, { FakeSession() }, scope, Backoff(50.milliseconds, 50.milliseconds),
            onStateChange = { history += it }, onError = { errors += it })
        m.start()
        m.await { it is LinkState.WaitingToRetry }
        assertEquals("refused", errors.first().message)
        m.stop()
    }

    /** A session that handshakes and fails at once, like a peer rejecting every frame. */
    private class FailingSession : LinkSession<String> {
        override suspend fun handshake() = "hello"
        override suspend fun run() = throw IOException("rejected")
        override fun close() {}
    }

    @Test
    fun `a session failing right after the handshake keeps backing off`() {
        // Issue #15: the backoff must not reset on every handshake, or a bad peer is retried every second forever
        val m = LinkManager({ FakeTransport() }, { FailingSession() }, scope, Backoff(20.milliseconds, 1.seconds),
            onStateChange = { history += it }, stableAfter = 10.seconds)
        m.start()
        repeat(4) { m.await { it is LinkState.WaitingToRetry } }
        m.stop()
        assertEquals(listOf(20, 40, 80, 160), history.filterIsInstance<LinkState.WaitingToRetry>().take(4).map { it.delay.inWholeMilliseconds.toInt() })
    }

    @Test
    fun `a stable session resets the backoff`() {
        val t = FakeTransport(failures = 2)
        val m = manager({ t }, stableAfter = Duration.ZERO); m.start()
        m.await { it is LinkState.Connected }
        sessions.first().end.completeExceptionally(IOException("dropped"))
        assertEquals(LinkState.WaitingToRetry(1, 50.milliseconds), m.await { it is LinkState.WaitingToRetry })
        m.stop()
    }

    @Test
    fun `a reconnectNow during one wait does not shorten the next`() {
        // Issue #14: a signal left in the conflated channel used to end the following wait at once
        val t = FakeTransport(failures = 100)
        val m = manager({ t }, initial = 300.milliseconds, max = 300.milliseconds); m.start()
        m.await { it is LinkState.WaitingToRetry }
        m.reconnectNow(); m.reconnectNow()
        m.await { it == LinkState.Connecting }
        m.await { it is LinkState.WaitingToRetry }
        val waitStarted = System.nanoTime()
        m.await { it == LinkState.Connecting }
        val waited = (System.nanoTime() - waitStarted) / 1_000_000
        m.stop()
        assertTrue(waited >= 200, "second wait lasted only $waited ms")
    }

    @Test
    fun `no state after stopped`() {
        // Issue #13: a transition racing stop() must not be published after Stopped
        repeat(100) {
            history.clear()
            val m = manager({ FakeTransport(failures = 1000) }, initial = 1.milliseconds, max = 1.milliseconds)
            m.start()
            Thread.sleep(2)
            m.stop()
            Thread.sleep(5)
            assertEquals(LinkState.Stopped, history.last(), "history: $history")
        }
    }
}
