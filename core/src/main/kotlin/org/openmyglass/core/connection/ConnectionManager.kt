package org.openmyglass.core.connection

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
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
 * The reconnection loop is the generic [LinkManager]; this class maps it to the companion protocol.
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

    private inner class CompanionSession(val link: CompanionLink) : LinkSession<GlassHello> {
        override suspend fun handshake(): GlassHello = link.handshake(handshakeTimeout).also { _glassInfo.value = null }
        override suspend fun run() {
            link.requestGlassInfo()
            link.events().collect { event ->
                if (event is CompanionEvent.GlassInfoReceived) _glassInfo.value = event.info
                _events.emit(event)
            }
        }
        override fun close() = link.close()
    }

    private val manager = LinkManager<GlassHello>(
        transport = { transport },
        open = { CompanionSession(linkFactory(it)) },
        scope = scope,
        backoff = backoff,
        onStateChange = { s -> map(s).let { onStateChange(it); _state.value = it } },
    )

    fun start() = manager.start()

    fun stop() = manager.stop()

    /** Skips the current retry wait, e.g. on a Bluetooth bond/ACL event or a firmware ping. */
    fun reconnectNow() = manager.reconnectNow()

    suspend fun requestGlassInfo() = connectedLink().requestGlassInfo()

    suspend fun sendTimeZone(id: String) = connectedLink().sendTimeZone(id)

    private fun connectedLink(): CompanionLink {
        val current = (manager.currentSession() as? CompanionSession)?.link
        check(_state.value is ConnectionState.Connected && current != null) { "not connected to Glass" }
        return current
    }

    private fun map(s: LinkState<GlassHello>): ConnectionState = when (s) {
        LinkState.Disconnected -> ConnectionState.Disconnected
        LinkState.Connecting -> ConnectionState.Connecting
        LinkState.Handshaking -> ConnectionState.Handshaking
        is LinkState.Connected -> ConnectionState.Connected(s.hello)
        is LinkState.WaitingToRetry -> ConnectionState.WaitingToRetry(s.attempt, s.delay)
        is LinkState.Unavailable -> ConnectionState.Disconnected
        LinkState.Stopped -> ConnectionState.Stopped
    }
}
