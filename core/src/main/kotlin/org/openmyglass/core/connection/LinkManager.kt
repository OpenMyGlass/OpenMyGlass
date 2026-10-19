package org.openmyglass.core.connection

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel as SignalChannel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import org.openmyglass.core.transport.Channel
import org.openmyglass.core.transport.Transport
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds
import kotlin.time.TimeSource

/** One connected session of a link: handshake, then [run] until the link ends (returns) or fails (throws). */
interface LinkSession<H> {
    suspend fun handshake(): H
    suspend fun run()
    fun close()
}

sealed interface LinkState<out H> {
    data object Disconnected : LinkState<Nothing>
    data object Connecting : LinkState<Nothing>
    data object Handshaking : LinkState<Nothing>
    data class Connected<H>(val hello: H) : LinkState<H>
    data class WaitingToRetry(val attempt: Int, val delay: Duration) : LinkState<Nothing>
    /** No transport right now (not configured, Bluetooth off, permission missing). */
    data class Unavailable(val reason: String) : LinkState<Nothing>
    data object Stopped : LinkState<Nothing>
}

/**
 * Keeps one link alive: connect → handshake → run → back off on any failure (architecture §4, §6).
 * [transport] is asked on every attempt and may return `null` when no transport is available, which
 * backs off without trying to connect. [onStateChange] sees every transition (the [state] flow may skip
 * short-lived ones).
 */
class LinkManager<H>(
    private val transport: () -> Transport?,
    private val open: (Channel) -> LinkSession<H>,
    private val scope: CoroutineScope,
    private val backoff: Backoff = Backoff(),
    private val onStateChange: (LinkState<H>) -> Unit = {},
    /** Why an attempt or session ended (for logs); the loop retries regardless. */
    private val onError: (Throwable) -> Unit = {},
    /** A session must stay up this long before the backoff resets (a peer failing right after the handshake keeps backing off). */
    private val stableAfter: Duration = 30.seconds,
) {
    private val _state = MutableStateFlow<LinkState<H>>(LinkState.Disconnected)
    val state: StateFlow<LinkState<H>> = _state.asStateFlow()

    private val wake = SignalChannel<Unit>(SignalChannel.CONFLATED)
    @Volatile private var job: Job? = null
    @Volatile private var session: LinkSession<H>? = null
    @Volatile private var stopped = false
    private val stateLock = Any()

    fun start() {
        if (job?.isActive == true) return
        stopped = false
        job = scope.launch { loop() }
    }

    fun stop() {
        // Under the state lock: no transition can be published after Stopped
        synchronized(stateLock) {
            stopped = true
            onStateChange(LinkState.Stopped)
            _state.value = LinkState.Stopped
        }
        job?.cancel()
        session?.close()
    }

    /** Skips the current wait (Bluetooth turned on, device back in range, firmware ping). */
    fun reconnectNow() {
        val s = _state.value
        if (s is LinkState.WaitingToRetry || s is LinkState.Unavailable) wake.trySend(Unit)
    }

    /** The current session while connected, for sending; null otherwise. */
    fun currentSession(): LinkSession<H>? = session.takeIf { _state.value is LinkState.Connected }

    private fun setState(s: LinkState<H>) = synchronized(stateLock) {
        if (stopped) return
        onStateChange(s)
        _state.value = s
    }

    private suspend fun loop() {
        var attempt = 0
        while (scope.isActive && !stopped) {
            val t = transport()
            if (t == null) {
                setState(LinkState.Unavailable("no transport"))
            } else {
                setState(LinkState.Connecting)
                var channel: Channel? = null
                var current: LinkSession<H>? = null
                try {
                    // Store the channel inside the block: withContext discards its result if we were cancelled
                    // meanwhile, and finally must still close a channel that connected late.
                    withContext(Dispatchers.IO) { channel = t.connect() }
                    current = open(checkNotNull(channel)).also { session = it }
                    setState(LinkState.Handshaking)
                    val hello = current.handshake()
                    setState(LinkState.Connected(hello))
                    val connectedAt = TimeSource.Monotonic.markNow()
                    try {
                        current.run()
                    } finally {
                        if (connectedAt.elapsedNow() >= stableAfter) {
                            backoff.reset()
                            attempt = 0
                        }
                    }
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    // refused, handshake failure, framing error or dropped link: retry below
                    onError(e)
                } finally {
                    current?.close()
                    runCatching { channel?.close() }
                    if (session === current) session = null
                }
            }
            if (stopped) break
            attempt++
            val delay = backoff.next()
            wake.tryReceive()   // a reconnectNow left over from an earlier wait must not cut this one short
            setState(LinkState.WaitingToRetry(attempt, delay))
            withTimeoutOrNull(delay) { wake.receive() }
        }
    }
}
