package org.openmyglass.core.bridge

import com.google.protobuf.ByteString
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.assertThrows
import org.openmyglass.core.companion.FakeGlass
import org.openmyglass.core.framing.DelimitedFraming
import org.openmyglass.protocol.v1.Bridge.Frame
import org.openmyglass.protocol.v1.Bridge.Hello
import org.openmyglass.protocol.v1.Bridge.Icon
import java.io.IOException
import kotlin.time.Duration.Companion.milliseconds
import kotlin.time.Duration.Companion.seconds

class BridgeLinkTest {

    // FakeGlass gives a loopback socket pair; here its Glass side speaks bridge frames.
    private val peer = FakeGlass.open()
    private val link = BridgeLink(peer.phoneChannel, appVersion = "0.1.0")

    private fun peerSend(frame: Frame) = peer.sendRaw(java.io.ByteArrayOutputStream().also {
        DelimitedFraming.writeFrame(it, frame.toByteArray())
    }.toByteArray())

    private fun peerReceive(): Frame = Frame.parseFrom(peer.receiveRaw())

    @AfterEach
    fun tearDown() = peer.close()

    @Test
    fun `handshake exchanges hellos`() = runBlocking {
        val glassHello = Hello.newBuilder().setProtocolVersion(1).setAppVersion("glass-0.1").addCapabilities("nav").build()
        val pending = async(Dispatchers.IO) { link.handshake() }
        val ours = peerReceive()
        assertEquals(1, ours.hello.protocolVersion)
        assertEquals("0.1.0", ours.hello.appVersion)
        peerSend(Frame.newBuilder().setHello(glassHello).build())
        assertEquals(glassHello, pending.await())
    }

    @Test
    fun `handshake times out on a silent peer`() = runBlocking {
        assertThrows<IOException> { runBlocking { link.handshake(timeout = 300.milliseconds) } }
        Unit
    }

    @Test
    fun `oversized frame is rejected`() = runBlocking {
        val big = Frame.newBuilder().setIcon(Icon.newBuilder().setPng(ByteString.copyFrom(ByteArray(70 * 1024)))).build()
        assertThrows<IllegalArgumentException> { runBlocking { link.send(big) } }
        Unit
    }

    @Test
    fun `incoming completes on disconnect`() = runBlocking {
        peerSend(Frame.newBuilder().setHello(Hello.newBuilder().setProtocolVersion(1)).build())
        peer.closeOutput()
        val frames = withTimeout(2.seconds) { link.incoming().toList() }
        assertEquals(1, frames.size)
    }
}
