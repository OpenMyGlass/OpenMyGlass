package org.openmyglass.core.companion

import org.openmyglass.core.framing.DelimitedFraming
import org.openmyglass.core.transport.Channel
import org.openmyglass.protocol.companion.Companion.Envelope
import java.io.Closeable
import java.net.ServerSocket
import java.net.Socket

/** Test helper: a loopback socket pair; the phone side is a [Channel], the Glass side is scripted by the test. */
class FakeGlass private constructor(private val glass: Socket, val phoneChannel: Channel) : Closeable {

    fun send(envelope: Envelope) = DelimitedFraming.writeFrame(glass.getOutputStream(), envelope.toByteArray())

    fun sendRaw(bytes: ByteArray) { glass.getOutputStream().write(bytes); glass.getOutputStream().flush() }

    /** Reads the next envelope sent by the phone, failing after [timeoutMs]. */
    fun receive(timeoutMs: Int = 2_000): Envelope {
        glass.soTimeout = timeoutMs
        val frame = DelimitedFraming.readFrame(glass.getInputStream()) ?: error("phone closed the link")
        return Envelope.parseFrom(frame)
    }

    /** Reads the next raw frame payload sent by the phone, failing after [timeoutMs]. */
    fun receiveRaw(timeoutMs: Int = 2_000): ByteArray {
        glass.soTimeout = timeoutMs
        return DelimitedFraming.readFrame(glass.getInputStream()) ?: error("phone closed the link")
    }

    /** True if the phone sent nothing within [timeoutMs]. */
    fun silentFor(timeoutMs: Int): Boolean {
        glass.soTimeout = timeoutMs
        return try { glass.getInputStream().read(); false } catch (_: java.net.SocketTimeoutException) { true }
    }

    fun closeOutput() = glass.shutdownOutput()

    override fun close() { glass.close(); phoneChannel.close() }

    companion object {
        fun open(): FakeGlass = ServerSocket(0).use { server ->
            val phone = Socket("127.0.0.1", server.localPort)
            val glass = server.accept()
            FakeGlass(glass, object : Channel {
                override val input = phone.getInputStream()
                override val output = phone.getOutputStream()
                override fun close() = phone.close()
            })
        }

        fun glassHello(sdk: Int = 19): Envelope = Envelope.newBuilder()
            .setVersion(655366)
            .setAndroidDeviceInfo(org.openmyglass.protocol.companion.Companion.AndroidDeviceInfo.newBuilder().setSdkVersion(sdk))
            .setGlassFeatureInfoG2C(org.openmyglass.protocol.companion.Companion.GlassFeatureInfo.newBuilder().setIsWallpaperFeatureEnabled(false))
            .build()
    }
}
