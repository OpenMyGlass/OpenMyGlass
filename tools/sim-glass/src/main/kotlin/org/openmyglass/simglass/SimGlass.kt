package org.openmyglass.simglass

import org.openmyglass.core.companion.CompanionConstants
import org.openmyglass.core.companion.GlassInfo
import org.openmyglass.core.framing.DelimitedFraming
import org.openmyglass.protocol.companion.Companion.AndroidDeviceInfo
import org.openmyglass.protocol.companion.Companion.Envelope
import org.openmyglass.protocol.companion.Companion.GlassFeatureInfo
import org.openmyglass.protocol.companion.Companion.GlassInfoResponse
import java.io.Closeable
import java.io.IOException
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.Collections
import kotlin.concurrent.thread

data class SimGlassConfig(
    val port: Int = 0,
    val glassSdkVersion: Int = 19,
    val version: Int = CompanionConstants.VERSION,
    val info: GlassInfo = GlassInfo(80, 16_000_000_000, 8_000_000_000, "Sim Glass", "XE24", false, 640, 360),
    /** Never send the handshake, to test the companion's timeout. */
    val silent: Boolean = false,
)

/**
 * A stand-in for the XE24 companion server (docs/protocol/handshake.md, messages.md) over TCP:
 * sends its handshake first, answers device-info requests, records time zones, and accepts
 * one client at a time like the firmware.
 */
class SimGlass(
    private val config: SimGlassConfig,
    private val log: (String) -> Unit = {},
) : Closeable {
    private lateinit var server: ServerSocket
    @Volatile private var client: Socket? = null
    @Volatile private var closed = false

    val receivedTimeZones: MutableList<String> = Collections.synchronizedList(mutableListOf())
    val clientVersions: MutableList<Int> = Collections.synchronizedList(mutableListOf())
    val port: Int get() = server.localPort

    /** Starts listening on loopback and returns the bound port. */
    fun start(): Int {
        server = ServerSocket(config.port, 50, InetAddress.getLoopbackAddress())
        thread(name = "sim-glass-accept", isDaemon = true) { acceptLoop() }
        log("sim-glass listening on 127.0.0.1:$port")
        return port
    }

    fun dropCurrentClient() {
        client?.close()
    }

    override fun close() {
        closed = true
        client?.close()
        if (::server.isInitialized) server.close()
    }

    private fun acceptLoop() {
        while (!closed) {
            val socket = try { server.accept() } catch (_: IOException) { return }
            if (client?.isClosed == false) {
                log("refusing a second client (one connection at a time)")
                socket.close()
                continue
            }
            client = socket
            thread(name = "sim-glass-client", isDaemon = true) { serve(socket) }
        }
    }

    private fun serve(socket: Socket) = socket.use { s ->
        try {
            if (config.silent) {
                log("client connected; staying silent")
                while (s.getInputStream().read() != -1) Unit
                return
            }
            send(s, envelope()
                .setAndroidDeviceInfo(AndroidDeviceInfo.newBuilder().setSdkVersion(config.glassSdkVersion))
                .setGlassFeatureInfoG2C(GlassFeatureInfo.newBuilder().setIsWallpaperFeatureEnabled(false)))
            val hello = read(s) ?: return
            clientVersions += hello.version
            log("client handshake: version ${CompanionConstants.majorOf(hello.version)}.${CompanionConstants.minorOf(hello.version)}")
            if (CompanionConstants.majorOf(hello.version) > CompanionConstants.majorOf(config.version)) {
                log("WARNING: companion major version is newer; real Glass would ask for an OTA update")
            }
            handle(s, hello)
            while (true) handle(s, read(s) ?: break)
            log("client disconnected")
        } catch (e: IOException) {
            log("client link closed: ${e.message}")
        }
    }

    private fun handle(s: Socket, env: Envelope) {
        if (env.hasTimezoneC2G()) {
            receivedTimeZones += env.timezoneC2G
            log("time zone: ${env.timezoneC2G}")
        }
        if (env.hasGlassInfoRequestC2G()) {
            log("glass info requested")
            send(s, envelope().setGlassInfoResponseG2C(config.info.toProto()))
        }
    }

    private fun envelope(): Envelope.Builder = Envelope.newBuilder()
        .setVersion(config.version)
        .setTimeMillis(System.currentTimeMillis())
        .setUptimeMillis(System.nanoTime() / 1_000_000)

    private fun send(s: Socket, builder: Envelope.Builder) =
        synchronized(s) { DelimitedFraming.writeFrame(s.getOutputStream(), builder.build().toByteArray()) }

    private fun read(s: Socket): Envelope? = DelimitedFraming.readFrame(s.getInputStream())?.let(Envelope::parseFrom)

    private fun GlassInfo.toProto(): GlassInfoResponse.Builder = GlassInfoResponse.newBuilder().also { b ->
        batteryLevel?.let(b::setBatteryLevel)
        storageTotalBytes?.let(b::setExternalStorageTotalBytes)
        storageAvailableBytes?.let(b::setExternalStorageAvailableBytes)
        deviceName?.let(b::setDeviceName)
        softwareVersion?.let(b::setSoftwareVersion)
        needSetup?.let(b::setNeedSetup)
        screenWidthPx?.let(b::setGlassScreenWidthPixels)
        screenHeightPx?.let(b::setGlassScreenHeightPixels)
    }
}
