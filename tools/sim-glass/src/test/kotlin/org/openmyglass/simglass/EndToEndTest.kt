package org.openmyglass.simglass

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.filterIsInstance
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.openmyglass.core.companion.CompanionEvent
import org.openmyglass.core.companion.GlassHello
import org.openmyglass.core.companion.PhoneState
import org.openmyglass.core.connection.Backoff
import org.openmyglass.core.connection.ConnectionManager
import org.openmyglass.core.connection.ConnectionState
import org.openmyglass.core.transport.TcpTransport
import java.net.Socket
import java.util.Collections
import kotlin.time.Duration
import kotlin.time.Duration.Companion.milliseconds
import kotlin.time.Duration.Companion.seconds

class EndToEndTest {

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val phone = PhoneState(34, "Europe/Rome", wifiConnected = true, interactive = true, gpsEnabled = true)
    private val history = Collections.synchronizedList(mutableListOf<ConnectionState>())
    private var sim: SimGlass? = null

    private fun startSim(config: SimGlassConfig = SimGlassConfig()): Pair<SimGlass, ConnectionManager> {
        val s = SimGlass(config).also { sim = it }
        val port = s.start()
        val m = ConnectionManager(
            TcpTransport("127.0.0.1", port), { phone }, scope,
            Backoff(initial = 100.milliseconds, max = 400.milliseconds), handshakeTimeout = 1.seconds,
            onStateChange = { history += it },
        )
        m.start()
        return s to m
    }

    private fun ConnectionManager.await(timeout: Duration = 3.seconds, predicate: (ConnectionState) -> Boolean) =
        runBlocking { withTimeout(timeout) { state.first(predicate) } }

    @AfterEach
    fun tearDown() { scope.cancel(); sim?.close() }

    @Test
    fun `connects and receives glass info`() {
        val (_, m) = startSim()
        val info = runBlocking {
            withTimeout(3.seconds) {
                val pending = async { m.events.filterIsInstance<CompanionEvent.GlassInfoReceived>().first() }
                assertEquals(ConnectionState.Connected(GlassHello(655366, 19, false)), m.state.first { it is ConnectionState.Connected })
                pending.await().info
            }
        }
        assertEquals(80, info.batteryLevel)
        assertEquals("Sim Glass", info.deviceName)
    }

    @Test
    fun `time zone reaches glass`() {
        val (s, m) = startSim()
        m.await { it is ConnectionState.Connected }
        runBlocking { m.sendTimeZone("America/New_York") }
        runBlocking { withTimeout(2.seconds) { while ("America/New_York" !in s.receivedTimeZones) delay(20) } }
        assertTrue("Europe/Rome" in s.receivedTimeZones, "handshake time zone")
    }

    @Test
    fun `reconnects after glass drops the link`() {
        val (s, m) = startSim()
        m.await { it is ConnectionState.Connected }
        s.dropCurrentClient()
        m.await { it is ConnectionState.WaitingToRetry }
        m.await(2.seconds) { it is ConnectionState.Connected }
        assertEquals(2, history.count { it is ConnectionState.Connected })
    }

    @Test
    fun `second client is refused`() {
        val (s, m) = startSim()
        m.await { it is ConnectionState.Connected }
        Socket("127.0.0.1", s.port).use { second ->
            second.soTimeout = 2_000
            assertEquals(-1, second.getInputStream().read(), "second client must be closed immediately")
        }
        assertTrue(m.state.value is ConnectionState.Connected)
    }

    @Test
    fun `silent glass is retried`() {
        val (_, m) = startSim(SimGlassConfig(silent = true))
        m.await { it is ConnectionState.WaitingToRetry }
        val connected = runBlocking { withTimeoutOrNull(2_500.milliseconds) { m.state.first { it is ConnectionState.Connected } } }
        assertNull(connected)
        assertTrue(history.count { it is ConnectionState.Handshaking } >= 2)
    }
}
