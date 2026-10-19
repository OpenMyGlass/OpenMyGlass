package org.openmyglass.simglass

import com.google.protobuf.ByteString
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test
import org.openmyglass.core.bridge.BridgeLink
import org.openmyglass.core.transport.TcpTransport
import org.openmyglass.protocol.v1.Bridge.Frame
import org.openmyglass.protocol.v1.Bridge.Icon
import org.openmyglass.protocol.v1.Bridge.NavState
import kotlin.time.Duration.Companion.seconds

class SimBridgeTest {
    private val sim = SimBridge()
    private val port = sim.start()

    @AfterEach
    fun tearDown() = sim.close()

    private suspend fun connect(): BridgeLink =
        BridgeLink(TcpTransport("127.0.0.1", port).connect(), "test").also { it.handshake() }

    private suspend fun until(cond: () -> Boolean) = withTimeout(2.seconds) { while (!cond()) delay(20) }

    @Test
    fun `records nav states`() = runBlocking {
        val link = connect()
        link.send(Frame.newBuilder().setNav(NavState.newBuilder().setDistanceM(120).setSource("osmand")).build())
        until { sim.navStates.isNotEmpty() }
        assertEquals(120, sim.navStates.last().distanceM)
        link.send(Frame.newBuilder().setNavEnded(org.openmyglass.protocol.v1.Bridge.NavEnded.getDefaultInstance()).build())
        until { sim.ended == 1 }
    }

    @Test
    fun `requests unknown icons once`() = runBlocking {
        val link = connect()
        val sha = ByteString.copyFrom(ByteArray(32) { 7 })
        val nav = Frame.newBuilder().setNav(NavState.newBuilder().setSource("gmaps").setIconSha256(sha)).build()
        link.send(nav)
        val request = withTimeout(2.seconds) { link.incoming().filter { it.hasIconRequest() }.first() }
        assertEquals(sha, request.iconRequest.sha256)
        link.send(Frame.newBuilder().setIcon(Icon.newBuilder().setSha256(sha).setPng(ByteString.copyFrom(byteArrayOf(1)))).build())
        until { sim.icons.contains("07".repeat(32)) }
        link.send(nav)
        until { sim.navStates.size == 2 }
        assertEquals(1, sim.iconRequestsSent)
    }

    @Test
    fun `accepts a second session after a drop`() = runBlocking {
        connect()
        until { sim.sessions == 1 }
        sim.dropCurrentClient()
        val second = connect()
        second.send(Frame.newBuilder().setNav(NavState.newBuilder().setSource("osmand")).build())
        until { sim.sessions == 2 && sim.navStates.isNotEmpty() }
    }
}
