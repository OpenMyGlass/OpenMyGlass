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

    private fun manager(transport: () -> Transport?, initial: Duration = 50.milliseconds, max: Duration = 200.milliseconds) =
        LinkManager(transport, { FakeSession().also { sessions += it } }, scope, Backoff(initial, max), onStateChange = { history += it })

    private fun LinkManager<String>.await(timeout: Duration = 3.seconds, p: (LinkState<String>) -> Boolean) =
        runBlocking { withTimeout(timeout) { state.first(p) } }

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
        m.await { it is LinkState.Connected && sessions.size == 2 }
        assertTrue(history.any { it is LinkState.WaitingToRetry })
    }
}
