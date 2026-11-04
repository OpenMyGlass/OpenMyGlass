package org.openmyglass.companion

import org.openmyglass.corebindings.CompanionCore
import org.openmyglass.corebindings.CoreEvent
import org.openmyglass.corebindings.LinkProfile
import org.openmyglass.corebindings.linkProfileFast
import java.io.IOException
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.Semaphore
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/** A byte stream the app opened. The core does not care what carries it. */
interface ByteLink {
    /** How to pull over this link. */
    val profile: LinkProfile

    /** Blocks until bytes arrive. Returns how many, or -1 at the end. */
    @Throws(IOException::class)
    fun read(buffer: ByteArray): Int

    @Throws(IOException::class)
    fun write(bytes: ByteArray)

    fun close()
}

/**
 * Wi-Fi, or USB through `adb reverse`. Bluetooth Classic would be one more [ByteLink], on Android
 * only: nothing shared may assume it, because iOS does not have it.
 */
class TcpLink(host: String, port: Int) : ByteLink {
    private val socket = Socket().apply {
        tcpNoDelay = true
        connect(InetSocketAddress(host, port), CONNECT_TIMEOUT_MS)
    }

    override val profile: LinkProfile = linkProfileFast()
    override fun read(buffer: ByteArray): Int = socket.getInputStream().read(buffer)
    override fun write(bytes: ByteArray) {
        socket.getOutputStream().apply {
            write(bytes)
            flush()
        }
    }

    override fun close() = socket.close()

    private companion object {
        const val CONNECT_TIMEOUT_MS = 5000
    }
}

/**
 * Keeps a link to the Glass while [open] is set, reconnecting after a loss: notes sync by
 * themselves whenever it comes back. The threads are the two `docs/companion-mobile.md` asks
 * for, plus one that supervises.
 *
 * This lives as long as the app's process. There is no background service yet.
 */
class LinkController(
    private val core: CompanionCore,
    private val freeBytes: () -> Long,
    private val onEvents: (List<CoreEvent>) -> Unit,
) {
    private val wake = Semaphore(0)

    @Volatile private var open: (() -> ByteLink)? = null

    @Volatile private var current: ByteLink? = null
    private var supervisor: Thread? = null

    /** Keep a link open with what [opener] returns. */
    @Synchronized
    fun connect(opener: () -> ByteLink) {
        open = opener
        if (supervisor?.isAlive != true) {
            supervisor = Thread({ supervise() }, "omg-link").apply { start() }
        }
    }

    /** Close the link and stay down. */
    fun disconnect() {
        open = null
        current?.close()
        wake.release()
    }

    /** Call after anything that may have queued bytes in the core. */
    fun poke() = wake.release()

    private fun supervise() {
        var backoffMs = MIN_BACKOFF_MS
        while (true) {
            val opener = open ?: break
            try {
                val link = opener()
                current = link
                if (open != null) {
                    runLink(link)
                    backoffMs = MIN_BACKOFF_MS
                }
            } catch (_: IOException) {
                // The next attempt is below.
            }
            current = null
            deliver(core.pump().events)
            if (open == null) break
            wake.tryAcquire(backoffMs, TimeUnit.MILLISECONDS)
            backoffMs = (backoffMs * 2).coerceAtMost(MAX_BACKOFF_MS)
        }
    }

    private fun runLink(link: ByteLink) {
        core.setFreeBytes(freeBytes().toULong())
        // Connecting is the owner's action, so a core with no bond may bond here.
        core.linkUp(link.profile, core.snapshot().bondedTo == null)
        val ended = AtomicBoolean(false)
        val reader = Thread({
            val buffer = ByteArray(READ_BUFFER)
            try {
                while (true) {
                    val n = link.read(buffer)
                    if (n < 0) break
                    core.onBytes(buffer.copyOf(n))
                    wake.release()
                }
            } catch (_: IOException) {
                // The link is gone: the same as the end of the stream.
            }
            ended.set(true)
            wake.release()
        }, "omg-read").apply { start() }

        try {
            while (!ended.get() && open != null) {
                wake.tryAcquire(PUMP_INTERVAL_MS, TimeUnit.MILLISECONDS)
                wake.drainPermits()
                val out = core.pump()
                if (out.outgoing.isNotEmpty()) link.write(out.outgoing)
                deliver(out.events)
                if (out.closeLink) break
            }
        } catch (_: IOException) {
            // Handled below like any other end.
        } finally {
            link.close()
            reader.join()
            core.linkDown()
        }
    }

    private fun deliver(events: List<CoreEvent>) {
        if (events.isNotEmpty()) onEvents(events)
    }

    private companion object {
        const val PUMP_INTERVAL_MS = 200L
        const val READ_BUFFER = 64 * 1024
        const val MIN_BACKOFF_MS = 1000L
        const val MAX_BACKOFF_MS = 30_000L
    }
}
