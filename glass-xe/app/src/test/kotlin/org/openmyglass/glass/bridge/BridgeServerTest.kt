package org.openmyglass.glass.bridge

import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.openmyglass.core.bridge.BridgeLink
import org.openmyglass.core.transport.TcpTransport
import org.openmyglass.protocol.v1.Bridge
import java.net.Socket
import java.util.Collections

class BridgeServerTest {

    private val events = Collections.synchronizedList(mutableListOf<String>())
    private val frames = Collections.synchronizedList(mutableListOf<Bridge.Frame>())
    private val server = BridgeServer(0, object : BridgeServer.FrameHandler {
        override fun onSessionStarted(sender: BridgeServer.Sender) { events += "started" }
        override fun onFrame(frame: Bridge.Frame) { frames += frame }
        override fun onSessionEnded() { events += "ended" }
    })
    private val port = server.start()

    @After
    fun tearDown() = server.close()

    private fun link() = BridgeLink(TcpTransport("127.0.0.1", port).connect(), "test")

    private fun await(condition: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 3_000
        while (!condition()) {
            if (System.currentTimeMillis() > deadline) throw AssertionError("condition not met in time")
            Thread.sleep(20)
        }
    }

    private fun nav(distance: Int) =
        Bridge.Frame.newBuilder().setNav(Bridge.NavState.newBuilder().setDistanceM(distance).setSource("osmand")).build()

    @Test
    fun `answers hello with capabilities nav and icons`() = runBlocking {
        val hello = link().handshake()
        assertEquals(1, hello.protocolVersion)
        assertTrue(hello.capabilitiesList.containsAll(listOf("nav", "icons")))
        await { "started" in events }
    }

    @Test
    fun `delivers frames in order`() = runBlocking {
        val link = link().also { it.handshake() }
        link.send(nav(300)); link.send(nav(200)); link.send(nav(100))
        await { frames.size == 3 }
        assertEquals(listOf(300, 200, 100), frames.map { it.nav.distanceM })
    }

    @Test
    fun `malformed frame ends the session but the server keeps accepting`() = runBlocking {
        Socket("127.0.0.1", port).use { raw ->
            raw.getOutputStream().write(ByteArray(6) { 0xFF.toByte() })
            raw.getOutputStream().flush()
            raw.soTimeout = 3_000
            // The server closes the socket; with unread bytes left the close may arrive as a reset.
            val closed = try { raw.getInputStream().read() == -1 } catch (_: java.net.SocketException) { true }
            assertTrue("server must close a session with a malformed frame", closed)
        }
        assertEquals(1, link().handshake().protocolVersion)
    }

    @Test
    fun `a new connection replaces the active session`() = runBlocking {
        link().handshake()
        await { events.size == 1 }
        val second = link().also { it.handshake() }
        await { events.size == 3 }
        assertEquals(listOf("started", "ended", "started"), events.toList())
        second.send(nav(50))
        await { frames.size == 1 }
    }

    @Test
    fun `a connection without hello does not replace the active session`() = runBlocking {
        val phone = link().also { it.handshake() }
        await { events.size == 1 }
        Socket("127.0.0.1", port).use { stranger ->
            stranger.getOutputStream().write(byteArrayOf(0x00))   // an empty frame: not a Hello
            stranger.getOutputStream().flush()
            stranger.soTimeout = 3_000
            val closed = try { stranger.getInputStream().read() == -1 } catch (_: java.net.SocketException) { true }
            assertTrue(closed)
        }
        phone.send(nav(42))
        await { frames.size == 1 }
        assertEquals(listOf("started"), events.toList())
    }

    @Test
    fun `a silent connection is dropped after the hello timeout`() {
        val quick = BridgeServer(0, object : BridgeServer.FrameHandler {
            override fun onSessionStarted(sender: BridgeServer.Sender) {}
            override fun onFrame(frame: Bridge.Frame) {}
            override fun onSessionEnded() {}
        }, 300)
        val p = quick.start()
        try {
            Socket("127.0.0.1", p).use { silent ->
                silent.soTimeout = 3_000
                val closed = try { silent.getInputStream().read() == -1 } catch (_: java.net.SocketException) { true }
                assertTrue(closed)
            }
        } finally { quick.close() }
    }
}
