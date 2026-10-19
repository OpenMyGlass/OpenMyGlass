package org.openmyglass.core.bridge

import com.google.protobuf.ByteString
import kotlinx.coroutines.CancellationException
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
    private var active: Session? = null
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

    /** A bridge session; [detach] only ends the session it is given (a late cleanup cannot end a newer one). */
    class Session internal constructor(internal val send: suspend (Frame) -> Unit, internal val onFailure: () -> Unit)

    /**
     * Starts a bridge session, replacing any previous one: the current state (and its icon) is sent right away.
     * [onFailure] runs when a send fails and the relay drops the session; the owner should close the link so it
     * reconnects (an open link the relay no longer feeds would leave Glass without updates).
     */
    suspend fun attach(onFailure: () -> Unit = {}, send: suspend (Frame) -> Unit): Session = lock.withLock {
        val session = Session(send, onFailure)
        active = session
        lastSent = null
        sentIcons.clear()
        // Glass may still show a card from before the reconnect: say explicitly that nothing is navigating.
        if (current == null) deliver(Frame.newBuilder().setNavEnded(Bridge.NavEnded.getDefaultInstance()).build())
        push()
        session
    }

    suspend fun detach(session: Session) = lock.withLock {
        if (active === session) active = null
    }

    suspend fun onIconRequest(sha256: ByteArray) = lock.withLock {
        val hex = sha256.toHex()
        icons(hex)?.let { png -> if (deliver(iconFrame(sha256, png))) sentIcons += hex }
        Unit
    }

    private suspend fun push() {
        if (active == null) return
        val update = current
        if (update == null) {
            if (lastSent != null) deliver(Frame.newBuilder().setNavEnded(Bridge.NavEnded.getDefaultInstance()).build())
            lastSent = null
            return
        }
        val state = update.toProto()
        if (state == lastSent) return
        update.iconSha256?.let { hex ->
            if (hex !in sentIcons) icons(hex)?.let { png -> if (deliver(iconFrame(hex.hexToBytes(), png))) sentIcons += hex }
        }
        if (deliver(Frame.newBuilder().setNav(state).build())) lastSent = state
    }

    /**
     * Sends on the active session. A failed send (link dropped, frame rejected) ends that session instead of
     * propagating: the collector must survive, and the next [attach] resends the current state.
     */
    private suspend fun deliver(frame: Frame): Boolean {
        val session = active ?: return false
        return try {
            session.send(frame)
            true
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            if (active === session) active = null
            lastSent = null
            runCatching { session.onFailure() }
            false
        }
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
