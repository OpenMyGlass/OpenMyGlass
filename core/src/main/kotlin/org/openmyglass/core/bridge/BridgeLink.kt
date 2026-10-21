package org.openmyglass.core.bridge

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.openmyglass.core.framing.DelimitedFraming
import org.openmyglass.core.transport.Channel
import org.openmyglass.core.transport.readFrameWithTimeout
import org.openmyglass.protocol.v1.Bridge.Frame
import org.openmyglass.protocol.v1.Bridge.Hello
import java.io.IOException
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds

/**
 * One session of the OpenMyGlass bridge protocol (`openmyglass.v1`) over a [Channel].
 * The bridge app on Glass is ours, so the phone speaks first.
 */
class BridgeLink(
    private val channel: Channel,
    private val appVersion: String,
    private val capabilities: List<String> = CAPABILITIES,
) {
    private val writeLock = Mutex()

    suspend fun handshake(timeout: Duration = 10.seconds): Hello {
        try {
            send(Frame.newBuilder().setHello(
                Hello.newBuilder().setProtocolVersion(PROTOCOL_VERSION).setAppVersion(appVersion).addAllCapabilities(capabilities)
            ).build())
            val frame = channel.readFrameWithTimeout(timeout, "bridge handshake")?.let(Frame::parseFrom)
                ?: throw IOException("Glass closed the bridge link during the handshake")
            if (!frame.hasHello()) throw IOException("expected Hello, got ${frame.bodyCase}")
            return frame.hello
        } catch (e: IOException) {
            close()
            throw e
        }
    }

    suspend fun send(frame: Frame) {
        val bytes = frame.toByteArray()
        require(bytes.size <= MAX_FRAME_BYTES) { "bridge frame of ${bytes.size} bytes exceeds $MAX_FRAME_BYTES" }
        writeLock.withLock { withContext(Dispatchers.IO) { DelimitedFraming.writeFrame(channel.output, bytes) } }
    }

    /** Frames from Glass until a clean end of stream (completes) or an I/O error (throws). */
    fun incoming(): Flow<Frame> = flow {
        while (true) {
            val bytes = withContext(Dispatchers.IO) { DelimitedFraming.readFrame(channel.input, MAX_FRAME_BYTES) } ?: break
            emit(Frame.parseFrom(bytes))
        }
    }

    fun close() {
        runCatching { channel.close() }
    }

    companion object {
        const val PROTOCOL_VERSION = 1
        /** RFCOMM service of the OpenMyGlass Glass app (secure, bonded devices only); SDP name [RFCOMM_NAME]. */
        val RFCOMM_UUID: java.util.UUID = java.util.UUID.fromString("8a0dcb95-c455-453f-87b5-d69ddc82b743")
        const val RFCOMM_NAME = "OpenMyGlass Bridge"
        /** Kept small for BLE (iOS path, architecture §10). */
        const val MAX_FRAME_BYTES = 64 * 1024
        /** "dictation-free": DictationStart.free is understood (transcript only, no reply). */
        val CAPABILITIES = listOf("nav", "icons", "dictation-free")
        /** Added to [CAPABILITIES] when the phone serves LocationSubscribe (docs/location.md). */
        const val CAPABILITY_LOCATION = "location"
    }
}
