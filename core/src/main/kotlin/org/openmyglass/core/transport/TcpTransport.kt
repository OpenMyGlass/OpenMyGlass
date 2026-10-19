package org.openmyglass.core.transport

import java.io.InputStream
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.Socket

/** TCP transport: used for the simulated Glass and, later, for the bridge app over the LAN. */
class TcpTransport(
    val host: String,
    val port: Int,
    val connectTimeoutMs: Int = 5_000,
) : Transport {
    override val kind = TransportKind.TCP

    override fun connect(): Channel {
        val socket = Socket()
        try {
            socket.tcpNoDelay = true
            socket.connect(InetSocketAddress(host, port), connectTimeoutMs)
        } catch (e: Exception) {
            socket.close()
            throw e
        }
        return SocketChannel(socket)
    }

    private class SocketChannel(private val socket: Socket) : Channel {
        override val input: InputStream = socket.getInputStream()
        override val output: OutputStream = socket.getOutputStream()
        override fun close() = socket.close()
    }
}
