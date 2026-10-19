package org.openmyglass.core.connection

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel as SignalChannel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import org.openmyglass.core.companion.CompanionEvent
import org.openmyglass.core.companion.CompanionLink
import org.openmyglass.core.companion.GlassHello
import org.openmyglass.core.companion.GlassInfo
import org.openmyglass.core.companion.PhoneState
import org.openmyglass.core.transport.Channel
import org.openmyglass.core.transport.Transport
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds

sealed interface ConnectionState {
    data object Disconnected : ConnectionState
    data object Connecting : ConnectionState
    data object Handshaking : ConnectionState
    data class Connected(val hello: GlassHello) : ConnectionState
    data class WaitingToRetry(val attempt: Int, val delay: Duration) : ConnectionState
    data object Stopped : ConnectionState
}

/**
 * Keeps one [CompanionLink] to Glass alive: connect, handshake, request device info once
 * (no polling, architecture §5.1/§6), forward events, and reconnect with [Backoff] on any failure.
 */
class ConnectionManager(
    private val transport: Transport,
    private val phone: () -> PhoneState,
    private val scope: CoroutineScope,
    private val backoff: Backoff = Backoff(),
    private val handshakeTimeout: Duration = 10.seconds,
    private val linkFactory: (Channel) -> CompanionLink = { CompanionLink(it, phone) },
    /** Called synchronously on every transition (unlike [state], which may skip short-lived states). */
    private val onStateChange: (ConnectionState) -> Unit = {},
) {
    private val _state = MutableStateFlow<ConnectionState>(ConnectionState.Disconnected)
    val state: StateFlow<ConnectionState> = _state.asStateFlow()

    private val _events = MutableSharedFlow<CompanionEvent>(extraBufferCapacity = 64)
    val events: SharedFlow<CompanionEvent> = _events.asSharedFlow()

    private val _glassInfo = MutableStateFlow<GlassInfo?>(null)
    /** Latest device info of the current session; kept for late subscribers ([events] does not replay). */
    val glassInfo: StateFlow<GlassInfo?> = _glassInfo.asStateFlow()

    private val wake = SignalChannel<Unit>(SignalChannel.CONFLATED)
    @Volatile private var job: Job? = null
    @Volatile private var link: CompanionLink? = null
    @Volatile private var stopped = false

    fun start() {
        if (job?.isActive == true) return
        stopped = false
        job = scope.launch { loop() }
    }

    fun stop() {
        stopped = true
        job?.cancel()
        link?.close()
        onStateChange(ConnectionState.Stopped)
        _state.value = ConnectionState.Stopped
    }

    /** Skips the current retry wait, e.g. on a Bluetooth bond/ACL event or a firmware ping. */
    fun reconnectNow() {
        if (_state.value is ConnectionState.WaitingToRetry) wake.trySend(Unit)
    }

    suspend fun requestGlassInfo() = connectedLink().requestGlassInfo()

    suspend fun sendTimeZone(id: String) = connectedLink().sendTimeZone(id)

    private fun connectedLink(): CompanionLink {
        val current = link
        check(_state.value is ConnectionState.Connected && current != null) { "not connected to Glass" }
        return current
    }

    private fun setState(s: ConnectionState) {
        if (stopped) return
        onStateChange(s)
        _state.value = s
    }

    private suspend fun loop() {
        var attempt = 0
        while (scope.isActive && !stopped) {
            setState(ConnectionState.Connecting)
            var channel: Channel? = null
            var current: CompanionLink? = null
            try {
                // Store the channel inside the block: withContext discards its result if we were cancelled
                // meanwhile (prompt cancellation), and finally must still close a channel that connected late.
                withContext(Dispatchers.IO) { channel = transport.connect() }
                current = linkFactory(checkNotNull(channel)).also { link = it }
                setState(ConnectionState.Handshaking)
                val hello = current.handshake(handshakeTimeout)
                backoff.reset()
                attempt = 0
                _glassInfo.value = null
                setState(ConnectionState.Connected(hello))
                current.requestGlassInfo()
                current.events().collect { event ->
                    if (event is CompanionEvent.GlassInfoReceived) _glassInfo.value = event.info
                    _events.emit(event)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // connect refused, handshake failure, framing error or dropped link: retry below
            } finally {
                current?.close()
                runCatching { channel?.close() }
                if (link === current) link = null
            }
            if (stopped) break
            attempt++
            val delay = backoff.next()
            setState(ConnectionState.WaitingToRetry(attempt, delay))
            withTimeoutOrNull(delay) { wake.receive() }
        }
    }
}
