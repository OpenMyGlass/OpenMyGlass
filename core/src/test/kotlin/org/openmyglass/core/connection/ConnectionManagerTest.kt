package org.openmyglass.core.connection

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
import org.openmyglass.core.companion.FakeGlass
import org.openmyglass.core.companion.GlassHello
import org.openmyglass.core.companion.PhoneState
import org.openmyglass.core.transport.Channel
import org.openmyglass.core.transport.Transport
import org.openmyglass.core.transport.TransportKind
import java.io.IOException
import java.util.Collections
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread
import kotlin.time.Duration
import kotlin.time.Duration.Companion.milliseconds
import kotlin.time.Duration.Companion.seconds

class ConnectionManagerTest {

    /** Each connect() takes the next scripted step: a failure, or a FakeGlass driven by a script on its own thread. */
    private class ScriptedTransport : Transport {
        override val kind = TransportKind.TCP
        val steps = ConcurrentLinkedQueue<() -> Channel>()
        val connects = AtomicInteger()
        val glasses = Collections.synchronizedList(mutableListOf<FakeGlass>())
        override fun connect(): Channel {
            connects.incrementAndGet()
            val step = steps.poll() ?: throw IOException("no more scripted connections")
            return step()
        }
        fun fail() = steps.add { throw IOException("refused") }
        fun glass(script: (FakeGlass) -> Unit) = steps.add {
            val g = FakeGlass.open(); glasses += g
            thread(isDaemon = true) { runCatching { script(g) } }
            g.phoneChannel
        }
    }

    private val phone = PhoneState(34, "Europe/Rome", wifiConnected = true, interactive = true, gpsEnabled = false)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val transport = ScriptedTransport()
    private val history = Collections.synchronizedList(mutableListOf<ConnectionState>(ConnectionState.Disconnected))

    private fun manager(initial: Duration = 50.milliseconds, max: Duration = 200.milliseconds, stableAfter: Duration = Duration.ZERO) =
        ConnectionManager(
            transport, { phone }, scope, Backoff(initial, max), handshakeTimeout = 300.milliseconds,
            onStateChange = { history += it }, stableAfter = stableAfter,
        )

    private val cursor = HistoryCursor(history)

    @Suppress("UnusedReceiverParameter")
    private fun ConnectionManager.await(timeout: Duration = 3.seconds, predicate: (ConnectionState) -> Boolean) =
        cursor.await(timeout, predicate)

    /** A Glass that handshakes, then keeps reading until the phone or the test closes the link. */
    private val goodGlass: (FakeGlass) -> Unit = { g ->
        g.send(FakeGlass.glassHello()); g.receive()
        while (true) g.receive(10_000)
    }

    @AfterEach
    fun tearDown() { scope.cancel(); transport.glasses.forEach { it.close() } }

    @Test
    fun `connects, handshakes and requests glass info`() {
        val gotInfoRequest = java.util.concurrent.CountDownLatch(1)
        transport.glass { g ->
            g.send(FakeGlass.glassHello()); g.receive()
            if (g.receive().hasGlassInfoRequestC2G()) gotInfoRequest.countDown()
        }
        val m = manager(); m.start()
        assertEquals(ConnectionState.Connected(GlassHello(655366, 19, false)), m.await { it is ConnectionState.Connected })
        assertTrue(gotInfoRequest.await(2, java.util.concurrent.TimeUnit.SECONDS))
        assertEquals(
            listOf(ConnectionState.Disconnected, ConnectionState.Connecting, ConnectionState.Handshaking,
                ConnectionState.Connected(GlassHello(655366, 19, false))),
            history.take(4),
        )
    }

    @Test
    fun `retries with backoff when connect fails`() {
        transport.fail(); transport.fail(); transport.glass(goodGlass)
        val m = manager(); m.start()
        m.await { it is ConnectionState.Connected }
        val waits = history.filterIsInstance<ConnectionState.WaitingToRetry>()
        assertEquals(listOf(ConnectionState.WaitingToRetry(1, 50.milliseconds), ConnectionState.WaitingToRetry(2, 100.milliseconds)), waits)
    }

    @Test
    fun `silent glass leads to retry, not hang`() {
        transport.glass { }  // never sends a handshake
        transport.glass(goodGlass)
        val m = manager(); m.start()
        m.await { it is ConnectionState.WaitingToRetry }
        m.await { it is ConnectionState.Connected }
    }

    @Test
    fun `corrupt frame leads to retry`() {
        transport.glass { g -> g.sendRaw(ByteArray(6) { 0xFF.toByte() }) }
        transport.glass(goodGlass)
        val m = manager(); m.start()
        m.await { it is ConnectionState.WaitingToRetry }
        m.await { it is ConnectionState.Connected }
        assertEquals(2, transport.connects.get())
    }

    @Test
    fun `disconnect after connected triggers reconnect and backoff reset`() {
        transport.fail()
        transport.glass { g -> g.send(FakeGlass.glassHello()); g.receive(); g.receive(); g.close() }
        transport.glass(goodGlass)
        val m = manager(); m.start()
        m.await { it is ConnectionState.Connected }
        m.await { it is ConnectionState.WaitingToRetry && history.count { s -> s is ConnectionState.Connected } == 1 }
        m.await { it is ConnectionState.Connected && history.count { s -> s is ConnectionState.Connected } == 2 }
        val waits = history.filterIsInstance<ConnectionState.WaitingToRetry>()
        assertEquals(ConnectionState.WaitingToRetry(1, 50.milliseconds), waits.last())
    }

    @Test
    fun `reconnectNow skips the wait`() {
        transport.fail(); transport.glass(goodGlass)
        val m = manager(initial = 10.seconds, max = 10.seconds); m.start()
        m.await { it is ConnectionState.WaitingToRetry }
        m.reconnectNow()
        m.await(timeout = 2.seconds) { it is ConnectionState.Connected }
    }

    @Test
    fun `stop closes the link and stays stopped`() {
        transport.glass(goodGlass)
        val m = manager(); m.start()
        m.await { it is ConnectionState.Connected }
        m.stop()
        m.await { it == ConnectionState.Stopped }
        Thread.sleep(400)
        assertEquals(ConnectionState.Stopped, m.state.value)
        assertEquals(1, transport.connects.get())
    }

    /** Channel that records whether it was closed. */
    private class TrackedChannel(private val inner: Channel) : Channel by inner {
        @Volatile var closed = false
        override fun close() { closed = true; inner.close() }
    }

    @Test
    fun `stop during a slow connect closes the late channel`() {
        val release = java.util.concurrent.CountDownLatch(1)
        val entered = java.util.concurrent.CountDownLatch(1)   // connect() is really running (not just the Connecting state)
        val tracked = java.util.concurrent.atomic.AtomicReference<TrackedChannel?>(null)   // written on an IO thread
        transport.steps.add {
            entered.countDown()
            release.await()
            val g = FakeGlass.open(); transport.glasses += g
            TrackedChannel(g.phoneChannel).also { tracked.set(it) }
        }
        val m = manager(); m.start()
        m.await { it == ConnectionState.Connecting }
        assertTrue(entered.await(2, java.util.concurrent.TimeUnit.SECONDS))
        m.stop()
        release.countDown()
        runBlocking { withTimeout(2.seconds) { while (tracked.get()?.closed != true) kotlinx.coroutines.delay(20) } }
    }

    @Test
    fun `restart while an old connect is pending does not break the new link`() {
        val release = java.util.concurrent.CountDownLatch(1)
        transport.steps.add {
            release.await()
            val g = FakeGlass.open(); transport.glasses += g
            thread(isDaemon = true) { runCatching { goodGlass(g) } }
            g.phoneChannel
        }
        transport.glass(goodGlass)
        val m = manager(); m.start()
        m.await { it == ConnectionState.Connecting }
        m.stop()
        m.start()
        m.await { it is ConnectionState.Connected }
        release.countDown()
        Thread.sleep(300)
        assertTrue(m.state.value is ConnectionState.Connected)
        runBlocking { m.requestGlassInfo() }
    }

    @Test
    fun `latest glass info is kept for late subscribers`() {
        transport.glass { g ->
            g.send(FakeGlass.glassHello()); g.receive(); g.receive()
            g.send(org.openmyglass.protocol.companion.Companion.Envelope.newBuilder().setVersion(655366)
                .setGlassInfoResponseG2C(org.openmyglass.protocol.companion.Companion.GlassInfoResponse.newBuilder().setBatteryLevel(77))
                .build())
            while (true) g.receive(10_000)
        }
        val m = manager(); m.start()
        m.await { it is ConnectionState.Connected }
        Thread.sleep(300) // the response has been emitted before anyone subscribes
        val info = runBlocking { withTimeout(2.seconds) { m.glassInfo.first { it != null } } }
        assertEquals(77, info?.batteryLevel)
    }
}
