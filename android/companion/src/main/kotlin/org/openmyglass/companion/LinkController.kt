package org.openmyglass.companion

import org.openmyglass.corebindings.CompanionCoreInterface
import org.openmyglass.corebindings.CoreEvent
import org.openmyglass.corebindings.LinkProfile
import org.openmyglass.corebindings.ReconnectBackoff
import org.openmyglass.corebindings.ReconnectBackoffInterface
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

    /** Blocks, within a limit, until what [write] took has had its chance to leave. */
    fun flush() = Unit

    fun close()
}

/** Wi-Fi, or USB through `adb reverse`. [BleLink] is the everyday one. */
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

    override fun close() {
        try {
            socket.close()
        } catch (_: IOException) {
            // It is closed as far as this app can tell.
        }
    }

    private companion object {
        const val CONNECT_TIMEOUT_MS = 5000
    }
}

/**
 * Opens a link and blocks until it is up. Fails with an [IOException]. It should give up, the same
 * way, once `cancelled` returns true: nobody wants this link any more.
 */
typealias LinkOpener = (cancelled: () -> Boolean) -> ByteLink

/**
 * Keeps a link to the Glass while [open] is set, reconnecting after a loss: notes sync by
 * themselves whenever it comes back. The threads are the two `docs/companion-mobile.md` asks
 * for, plus one that supervises, and one that polls so a note changing with no link still shows.
 *
 * This lives as long as the app's process. There is no background service yet.
 *
 * [onEvents] gets the link the events came from, or null once it has ended. It is also called with
 * no events each time an attempt ends, because a link that simply dropped has no event of its own
 * and the screen must still stop saying it is connected.
 * [onOpenFailed] says why the wanted link did not open, with the id [connect] returned for it. It and
 * [onEvents] run on the link's threads, so check [wants] once on the screen's thread.
 */
class LinkController(
    private val core: CompanionCoreInterface,
    private val freeBytes: () -> Long,
    private val backoff: ReconnectBackoffInterface = ReconnectBackoff(),
    private val onOpenFailed: (request: Long, why: String) -> Unit = { _, _ -> },
    private val flushLimitMs: Long = FLUSH_LIMIT_MS,
    private val onEvents: (events: List<CoreEvent>, from: ByteLink?) -> Unit,
) {
    /** Ends the wait between pumps. */
    private val wake = Semaphore(0)

    /** Ends the wait between attempts. Only the owner asking for something does that. */
    private val retry = Semaphore(0)

    /** One per [connect], so that asking again with the same opener is still a new request. */
    private class Request(val id: Long, val opener: LinkOpener)

    @Volatile private var open: Request? = null
    private var requests = 0L

    @Volatile private var current: ByteLink? = null
    private var supervisor: Thread? = null

    /** The link [disconnectAfterFlush] let finish, and until when, in [System.nanoTime]. */
    @Volatile private var flushing: Pair<ByteLink, Long>? = null

    /** The link that is open now, whatever its phase, to tell what carries it. */
    val running: ByteLink? get() = current

    @Volatile private var closed = false
    private val poller = Thread({ poll() }, "omg-poll").apply {
        isDaemon = true
        start()
    }

    /**
     * Keep a link open with what [opener] returns, instead of any link there is now. Returns the
     * request's id, which [wants] and [onOpenFailed] use.
     */
    @Synchronized
    fun connect(opener: LinkOpener): Long {
        val request = Request(++requests, opener)
        open = request
        endCurrent()
        if (supervisor?.isAlive != true) {
            supervisor = Thread({ supervise() }, "omg-link").apply { start() }
        }
        retry.release()
        return request.id
    }

    /** The link [connect] returned [request] for is still the one wanted. */
    fun wants(request: Long) = open?.id == request

    /** Close the link and stay down. */
    fun disconnect() {
        open = null
        endCurrent()
        retry.release()
    }

    /**
     * Stay down, after writing what the core has queued. A scan in progress stops at once. A
     * [connect] or [disconnect] meanwhile waits for the flush, for up to [flushLimitMs], since what
     * is queued may be an Unpair the Glass has to see.
     */
    fun disconnectAfterFlush() {
        current?.let { flushing = it to System.nanoTime() + flushLimitMs * 1_000_000 }
        open = null
        wake.release()
        retry.release()
    }

    /** Closes the current link, unless it is flushing: then only once its time is up. */
    private fun endCurrent() {
        val link = current ?: return
        val (flushed, until) = flushing ?: (null to 0L)
        val leftMs = (until - System.nanoTime()) / 1_000_000
        if (flushed !== link || leftMs <= 0) {
            link.close()
            return
        }
        // It ends by itself after the flush, and the supervisor then takes the new request.
        Thread({
            Thread.sleep(leftMs)
            if (current === link) link.close()
        }, "omg-flush-limit").apply {
            isDaemon = true
            start()
        }
    }

    /** Disconnect, and stop polling for good. */
    fun close() {
        closed = true
        poller.interrupt()
        disconnect()
    }

    private fun poll() {
        try {
            while (!closed) {
                Thread.sleep(POLL_INTERVAL_MS)
                val events = core.poll()
                if (events.isNotEmpty()) onEvents(events, null)
            }
        } catch (_: InterruptedException) {
            // Closed.
        }
    }

    /** Call after anything that may have queued bytes in the core. */
    fun poke() = wake.release()

    private fun supervise() {
        while (true) {
            // Under the lock, so that a connect() either sees this thread gone or is seen here.
            val request = synchronized(this) {
                open.also { if (it == null) supervisor = null }
            } ?: return
            var lastedMs = 0L
            openLink(request)?.let { link ->
                val started = System.nanoTime()
                // Before the check: connect() sets `open` and then closes `current`, so a link
                // nobody wants any more is either not run here or closed there.
                current = link
                if (open === request) runLink(link, request) else link.close()
                current = null
                lastedMs = (System.nanoTime() - started) / 1_000_000
            }
            onEvents(core.pump().events, null)
            // Emptied before the check: a request after the check ends the wait.
            retry.drainPermits()
            if (open === request) {
                retry.tryAcquire(backoff.nextWaitMs(lastedMs.toULong()).toLong(), TimeUnit.MILLISECONDS)
            }
            if (open !== request) backoff.reset()
        }
    }

    private fun openLink(request: Request): ByteLink? = try {
        request.opener { open !== request }
    } catch (e: IOException) {
        openFailed(request, e)
    } catch (e: RuntimeException) {
        // The framework's way of saying the same thing, and no reason to lose the app.
        openFailed(request, e)
    }

    private fun openFailed(request: Request, e: Exception): ByteLink? {
        if (open === request) onOpenFailed(request.id, e.message ?: e.toString())
        return null
    }

    private fun runLink(link: ByteLink, request: Request) {
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
            while (!ended.get()) {
                wake.tryAcquire(PUMP_INTERVAL_MS, TimeUnit.MILLISECONDS)
                wake.drainPermits()
                // Read before the pump, so what was queued before the owner stopped, or asked
                // for another link, still goes.
                val stopping = open !== request
                val out = core.pump()
                if (out.outgoing.isNotEmpty()) link.write(out.outgoing)
                deliver(out.events, link)
                if (out.closeLink || stopping) {
                    link.flush()
                    break
                }
            }
        } catch (_: IOException) {
            // Handled below like any other end.
        } finally {
            link.close()
            reader.join()
            core.linkDown()
        }
    }

    private fun deliver(events: List<CoreEvent>, from: ByteLink) {
        if (events.isNotEmpty()) onEvents(events, from)
    }

    private companion object {
        /** Above BleLink's own limits on a flush, so that it gives up before this does. */
        const val FLUSH_LIMIT_MS = 5_000L
        const val PUMP_INTERVAL_MS = 200L
        const val POLL_INTERVAL_MS = 500L
        const val READ_BUFFER = 64 * 1024
    }
}
