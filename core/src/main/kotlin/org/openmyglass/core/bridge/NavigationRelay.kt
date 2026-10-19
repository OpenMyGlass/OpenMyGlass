package org.openmyglass.core.bridge

import com.google.protobuf.ByteString
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.openmyglass.core.nav.Maneuver
import org.openmyglass.core.nav.NavUpdate
import org.openmyglass.core.nav.NavigationSource
import org.openmyglass.protocol.v1.Bridge
import org.openmyglass.protocol.v1.Bridge.Frame

/**
 * Forwards navigation to Glass over the bridge: picks the first source (in priority order) that is
 * navigating, sends a NavState only when it changes, NavEnded once when navigation stops, and each
 * maneuver icon once per bridge session, before the first state that references it (battery rules,
 * architecture §6).
 */
class NavigationRelay(
    sources: List<NavigationSource>,
    private val icons: (sha256Hex: String) -> ByteArray?,
    private val scope: CoroutineScope,
) {
    private val lock = Mutex()
    private var send: (suspend (Frame) -> Unit)? = null
    private var current: NavUpdate? = null
    private var lastSent: Bridge.NavState? = null
    private val sentIcons = mutableSetOf<String>()

    init {
        scope.launch {
            combine(sources.map { it.updates }) { updates -> updates.firstOrNull { it != null } }
                .distinctUntilChanged()
                .collect { update -> lock.withLock { current = update; push() } }
        }
    }

    /** Starts a bridge session: the current state (and its icon) is sent right away. */
    fun attach(send: suspend (Frame) -> Unit) {
        scope.launch {
            lock.withLock {
                this@NavigationRelay.send = send
                lastSent = null
                sentIcons.clear()
                push()
            }
        }
    }

    fun detach() {
        scope.launch { lock.withLock { send = null } }
    }

    suspend fun onIconRequest(sha256: ByteArray) = lock.withLock {
        val hex = sha256.toHex()
        icons(hex)?.let { png -> send?.invoke(iconFrame(sha256, png)); sentIcons += hex }
        Unit
    }

    private suspend fun push() {
        val out = send ?: return
        val update = current
        if (update == null) {
            if (lastSent != null) out(Frame.newBuilder().setNavEnded(Bridge.NavEnded.getDefaultInstance()).build())
            lastSent = null
            return
        }
        val state = update.toProto()
        if (state == lastSent) return
        update.iconSha256?.let { hex ->
            if (hex !in sentIcons) icons(hex)?.let { png -> out(iconFrame(hex.hexToBytes(), png)); sentIcons += hex }
        }
        out(Frame.newBuilder().setNav(state).build())
        lastSent = state
    }

    private fun iconFrame(sha256: ByteArray, png: ByteArray) = Frame.newBuilder()
        .setIcon(Bridge.Icon.newBuilder().setSha256(ByteString.copyFrom(sha256)).setPng(ByteString.copyFrom(png)))
        .build()

    private fun NavUpdate.toProto(): Bridge.NavState = Bridge.NavState.newBuilder().also { b ->
        b.maneuver = toProto(maneuver)
        b.source = source
        distanceMeters?.let { b.distanceM = it }
        instruction?.let { b.instruction = it }
        street?.let { b.street = it }
        eta?.let { b.eta = it }
        iconSha256?.let { b.iconSha256 = ByteString.copyFrom(it.hexToBytes()) }
    }.build()

    companion object {
        /** core [Maneuver] → proto: same names, proto numbers are ordinal + 1 and UNKNOWN is 0. */
        fun toProto(m: Maneuver): Bridge.Maneuver =
            if (m == Maneuver.UNKNOWN) Bridge.Maneuver.MANEUVER_UNKNOWN else Bridge.Maneuver.valueOf(m.name)

        private fun ByteArray.toHex() = joinToString("") { "%02x".format(it) }
        private fun String.hexToBytes() = chunked(2).map { it.toInt(16).toByte() }.toByteArray()
    }
}
