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
        override fun onSessionEnded(sender: BridgeServer.Sender) { events += "ended" }
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
            override fun onSessionEnded(sender: BridgeServer.Sender) {}
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

    @Test
    fun `sessions from two channels replace each other`() = runBlocking {
        // Two TCP channels stand in for TCP + RFCOMM: one active session across all of them
        val a = TcpServerChannel(0)
        val b = TcpServerChannel(0)
        val log = Collections.synchronizedList(mutableListOf<String>())
        val multi = BridgeServer(listOf(a, b), object : BridgeServer.FrameHandler {
            override fun onSessionStarted(sender: BridgeServer.Sender) { log += "started" }
            override fun onFrame(frame: Bridge.Frame) {}
            override fun onSessionEnded(sender: BridgeServer.Sender) { log += "ended" }
        }, BridgeServer.DEFAULT_HELLO_TIMEOUT_MS)
        multi.start()
        try {
            BridgeLink(TcpTransport("127.0.0.1", a.localPort()).connect(), "test").handshake()
            await { log.size == 1 }
            BridgeLink(TcpTransport("127.0.0.1", b.localPort()).connect(), "test").handshake()
            await { log.size == 3 }
            assertEquals(listOf("started", "ended", "started"), log.toList())
        } finally { multi.close() }
    }

    @Test
    fun `a failing channel is reopened`() = runBlocking {
        // e.g. Bluetooth turned off and on again on Glass: the RFCOMM listener must come back
        val tcp = TcpServerChannel(0)
        val flaky = object : ServerChannel {
            var failures = 1
            override fun open() = tcp.open()
            override fun accept(): ServerChannel.Connection {
                if (failures-- > 0) { tcp.close(); throw java.io.IOException("adapter off") }
                return tcp.accept()
            }
            override fun close() = tcp.close()
        }
        val log = Collections.synchronizedList(mutableListOf<String>())
        val retrying = BridgeServer(listOf(flaky), object : BridgeServer.FrameHandler {
            override fun onSessionStarted(sender: BridgeServer.Sender) { log += "started" }
            override fun onFrame(frame: Bridge.Frame) {}
            override fun onSessionEnded(sender: BridgeServer.Sender) {}
        }, BridgeServer.DEFAULT_HELLO_TIMEOUT_MS, 50)
        retrying.start()
        try {
            await { tcp.localPort() > 0 && flaky.failures < 0 }
            BridgeLink(TcpTransport("127.0.0.1", tcp.localPort()).connect(), "test").handshake()
            await { log.size == 1 }
        } finally { retrying.close() }
    }

    private fun rawHello(version: Int) = java.io.ByteArrayOutputStream().also {
        org.openmyglass.core.framing.DelimitedFraming.writeFrame(it, Bridge.Frame.newBuilder()
            .setHello(Bridge.Hello.newBuilder().setProtocolVersion(version)).build().toByteArray())
    }.toByteArray()

    private fun closedByPeer(socket: Socket): Boolean {
        socket.soTimeout = 3_000
        return try { socket.getInputStream().read() == -1 } catch (_: java.net.SocketException) { true }
    }

    @Test
    fun `a hello with an unknown protocol version is rejected`() {
        // Issue #26
        Socket("127.0.0.1", port).use { s ->
            s.getOutputStream().write(rawHello(2)); s.getOutputStream().flush()
            assertTrue(closedByPeer(s))
        }
        Thread.sleep(200)
        assertEquals(emptyList<String>(), events.toList())
    }

    @Test
    fun `a hello completing after close does not start a session`() {
        // Issue #21: a zombie session on a closed server would keep the phone "connected" to nothing
        Socket("127.0.0.1", port).use { s ->
            Thread.sleep(200)                       // accepted, waiting for the Hello
            server.close()
            s.getOutputStream().write(rawHello(1)); s.getOutputStream().flush()
            assertTrue(closedByPeer(s))
        }
        assertEquals(emptyList<String>(), events.toList())
    }

    @Test
    fun `a channel reopened while closing is closed again`() {
        // Issue #22: close() racing a reopen must not leave the channel listening
        val tcp = TcpServerChannel(0)
        lateinit var racing: BridgeServer
        val channel = object : ServerChannel {
            var opens = 0
            var failures = 1
            override fun open() { if (++opens == 2) racing.close(); tcp.open() }   // close() lands just before the reopen
            override fun accept(): ServerChannel.Connection {
                if (failures-- > 0) throw java.io.IOException("adapter off")
                return tcp.accept()
            }
            override fun close() = tcp.close()
        }
        racing = BridgeServer(listOf(channel), object : BridgeServer.FrameHandler {
            override fun onSessionStarted(sender: BridgeServer.Sender) {}
            override fun onFrame(frame: Bridge.Frame) {}
            override fun onSessionEnded(sender: BridgeServer.Sender) {}
        }, BridgeServer.DEFAULT_HELLO_TIMEOUT_MS, 20)
        racing.start()
        await { channel.opens >= 2 }
        Thread.sleep(200)
        val stillListening = try { Socket("127.0.0.1", tcp.localPort()).close(); true } catch (_: java.io.IOException) { false }
        assertTrue("channel still listening after close", !stillListening)
    }

    @Test
    fun `a busy tcp port is retried instead of failing`() {
        // Issue #24: a port still held (e.g. by the previous service instance) must not crash the app
        val holder = java.net.ServerSocket(0)
        val busyPort = holder.localPort
        val log = Collections.synchronizedList(mutableListOf<String>())
        val retrying = BridgeServer(listOf<ServerChannel>(TcpServerChannel(busyPort)), object : BridgeServer.FrameHandler {
            override fun onSessionStarted(sender: BridgeServer.Sender) { log += "started" }
            override fun onFrame(frame: Bridge.Frame) {}
            override fun onSessionEnded(sender: BridgeServer.Sender) {}
        }, BridgeServer.DEFAULT_HELLO_TIMEOUT_MS, 50)
        try {
            assertEquals(-1, retrying.start())
            holder.close()
            await { runCatching { runBlocking { BridgeLink(TcpTransport("127.0.0.1", busyPort).connect(), "test").handshake() } }.isSuccess }
            await { log.size == 1 }
        } finally { retrying.close(); holder.close() }
    }
}
