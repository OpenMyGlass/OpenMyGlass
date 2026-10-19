package org.openmyglass.simglass

import org.openmyglass.core.framing.DelimitedFraming
import org.openmyglass.protocol.v1.Bridge.Frame
import org.openmyglass.protocol.v1.Bridge.Hello
import org.openmyglass.protocol.v1.Bridge.IconRequest
import org.openmyglass.protocol.v1.Bridge.NavState
import java.io.Closeable
import java.io.IOException
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.Collections
import kotlin.concurrent.thread

/**
 * A stand-in for the OpenMyGlass bridge app on Glass (protocol `openmyglass.v1`) over TCP: answers the
 * phone's Hello, records navigation frames and icons, asks once for icons it does not have, and accepts
 * one client at a time.
 */
class SimBridge(private val port: Int = 0, private val log: (String) -> Unit = {}) : Closeable {
    private lateinit var server: ServerSocket
    @Volatile private var client: Socket? = null
    @Volatile private var closed = false

    val navStates: MutableList<NavState> = Collections.synchronizedList(mutableListOf())
    private val iconSet: MutableSet<String> = Collections.synchronizedSet(mutableSetOf())
    private val requested: MutableSet<String> = Collections.synchronizedSet(mutableSetOf())
    val icons: Set<String> get() = synchronized(iconSet) { iconSet.toSet() }
    @Volatile var ended = 0; private set
    @Volatile var sessions = 0; private set
    @Volatile var iconRequestsSent = 0; private set

    fun start(): Int {
        server = ServerSocket(port, 50, InetAddress.getLoopbackAddress())
        thread(name = "sim-bridge-accept", isDaemon = true) { acceptLoop() }
        log("sim-bridge listening on 127.0.0.1:${server.localPort}")
        return server.localPort
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
            if (client?.isClosed == false) { log("refusing a second client"); socket.close(); continue }
            client = socket
            thread(name = "sim-bridge-client", isDaemon = true) { serve(socket) }
        }
    }

    private fun serve(s: Socket) = s.use {
        try {
            val hello = read(s) ?: return
            if (!hello.hasHello()) { log("expected Hello, got ${hello.bodyCase}"); return }
            log("phone hello: protocol ${hello.hello.protocolVersion}, app ${hello.hello.appVersion}, ${hello.hello.capabilitiesList}")
            send(s, Frame.newBuilder().setHello(Hello.newBuilder().setProtocolVersion(1).setAppVersion("sim-bridge").addCapabilities("nav").addCapabilities("icons")).build())
            sessions++
            while (true) handle(s, read(s) ?: break)
            log("phone disconnected")
        } catch (e: IOException) {
            log("bridge link closed: ${e.message}")
        }
    }

    private fun handle(s: Socket, frame: Frame) {
        when {
            frame.hasNav() -> {
                val nav = frame.nav
                navStates += nav
                log("nav: ${nav.maneuver} ${if (nav.hasDistanceM()) "${nav.distanceM} m" else "-"} | ${nav.instruction} | ${nav.street} | eta ${nav.eta} | ${nav.source}" +
                    if (nav.hasIconSha256()) " | icon ${hex(nav.iconSha256.toByteArray()).take(12)}" else "")
                if (nav.hasIconSha256()) {
                    val h = hex(nav.iconSha256.toByteArray())
                    if (h !in iconSet && requested.add(h)) {
                        send(s, Frame.newBuilder().setIconRequest(IconRequest.newBuilder().setSha256(nav.iconSha256)).build())
                        iconRequestsSent++
                    }
                }
            }
            frame.hasNavEnded() -> { ended++; log("nav ended") }
            frame.hasIcon() -> {
                iconSet += hex(frame.icon.sha256.toByteArray())
                log("icon ${hex(frame.icon.sha256.toByteArray()).take(12)} (${frame.icon.png.size()} bytes)")
            }
            else -> log("ignored ${frame.bodyCase}")
        }
    }

    private fun send(s: Socket, frame: Frame) = synchronized(s) { DelimitedFraming.writeFrame(s.getOutputStream(), frame.toByteArray()) }
    private fun read(s: Socket): Frame? = DelimitedFraming.readFrame(s.getInputStream())?.let(Frame::parseFrom)
    private fun hex(b: ByteArray) = b.joinToString("") { "%02x".format(it) }
}
