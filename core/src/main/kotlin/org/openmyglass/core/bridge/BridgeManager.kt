package org.openmyglass.core.bridge

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.StateFlow
import org.openmyglass.core.connection.Backoff
import org.openmyglass.core.connection.LinkManager
import org.openmyglass.core.connection.LinkSession
import org.openmyglass.core.connection.LinkState
import org.openmyglass.core.transport.Channel
import org.openmyglass.core.transport.Transport
import org.openmyglass.protocol.v1.Bridge

/**
 * Keeps the bridge to the OpenMyGlass Glass app alive and feeds it from [relay]: each connected session is
 * attached to the relay (which resends the current state), icon requests from Glass are answered, and the
 * session is detached when the link ends. Reconnection and backoff come from [LinkManager].
 */
class BridgeManager(
    transport: () -> Transport?,
    private val relay: NavigationRelay,
    private val appVersion: String,
    scope: CoroutineScope,
    backoff: Backoff = Backoff(),
    onStateChange: (LinkState<Bridge.Hello>) -> Unit = {},
    onError: (Throwable) -> Unit = {},
) {
    private inner class Session(private val link: BridgeLink) : LinkSession<Bridge.Hello> {
        override suspend fun handshake(): Bridge.Hello = link.handshake()
        override suspend fun run() {
            // A failed send drops the relay session: close the link so LinkManager reconnects and reattaches
            val attached = relay.attach(onFailure = link::close, send = link::send)
            try {
                link.incoming().collect { frame ->
                    if (frame.hasIconRequest()) relay.onIconRequest(frame.iconRequest.sha256.toByteArray())
                }
            } finally {
                relay.detach(attached)
            }
        }
        override fun close() = link.close()
    }

    private val manager = LinkManager(transport, { channel: Channel -> Session(BridgeLink(channel, appVersion)) },
        scope, backoff, onStateChange, onError)

    val state: StateFlow<LinkState<Bridge.Hello>> = manager.state

    fun start() = manager.start()

    fun stop() = manager.stop()

    /** Skips the current retry wait (Bluetooth turned on, Glass back in range). */
    fun reconnectNow() = manager.reconnectNow()
}
