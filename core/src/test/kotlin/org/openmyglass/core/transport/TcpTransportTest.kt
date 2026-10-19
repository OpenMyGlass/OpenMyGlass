package org.openmyglass.core.transport

import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.assertThrows
import java.io.IOException
import java.net.ServerSocket
import kotlin.concurrent.thread

class TcpTransportTest {

    @Test
    fun `tcp transport exchanges bytes`() {
        ServerSocket(0).use { server ->
            val echo = thread {
                server.accept().use { s ->
                    val buf = ByteArray(3)
                    var n = 0
                    while (n < 3) n += s.getInputStream().read(buf, n, 3 - n)
                    s.getOutputStream().write(buf)
                    s.getOutputStream().flush()
                }
            }
            val transport = TcpTransport("127.0.0.1", server.localPort)
            assertEquals(TransportKind.TCP, transport.kind)
            transport.connect().use { ch ->
                ch.output.write(byteArrayOf(1, 2, 3))
                ch.output.flush()
                val back = ByteArray(3)
                var n = 0
                while (n < 3) n += ch.input.read(back, n, 3 - n)
                assertArrayEquals(byteArrayOf(1, 2, 3), back)
            }
            echo.join(2_000)
        }
    }

    @Test
    fun `tcp transport fails fast when nothing listens`() {
        val port = ServerSocket(0).use { it.localPort }
        val start = System.nanoTime()
        assertThrows<IOException> { TcpTransport("127.0.0.1", port, connectTimeoutMs = 1_000).connect() }
        assert((System.nanoTime() - start) / 1_000_000 < 2_000)
    }
}
