package org.openmyglass.android.nav

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.openmyglass.core.nav.NavUpdate
import org.openmyglass.core.nav.NavUpdateReducer
import org.openmyglass.core.nav.NavigationSource
import org.openmyglass.core.nav.OsmAndNotificationParser

/**
 * Navigation updates from OsmAnd's public AIDL API (docs/navigation-sources.md).
 *
 * Threading: AIDL callbacks arrive on binder threads and the AIDL callback interface is synchronous, so
 * calls into OsmAnd are never made while holding [lock] (OsmAnd may call us back during a register call).
 * Every connection gets a new generation; callbacks from an older generation are ignored.
 */
class OsmAndNavigationSource(private val binder: OsmAndBinder) : NavigationSource {
    override val name = "osmand"

    private val lock = Any()
    private val reducer = NavUpdateReducer(name)
    private var api: OsmAndApi? = null
    private var registrations: List<Long> = emptyList()
    private var generation = 0
    private var started = false
    /** Instruction (with street) and ETA from OsmAnd's own notification, which its AIDL API lacks. */
    private var extra = OsmAndNotificationParser.Extra(null, null)

    private val _available = MutableStateFlow(false)
    override val available: StateFlow<Boolean> = _available.asStateFlow()

    private val _needsAuthorization = MutableStateFlow(false)
    /**
     * True when OsmAnd refused our registration: it returns -1 until the user enables OpenMyGlass in
     * OsmAnd (5.4: Menu → Plugins → OpenMyGlass; newer builds: the "navigation" permission group,
     * see AndroidOsmAndBinder.permissionRequestIntent). Source: OsmandAidlServiceV2.getApi.
     */
    val needsAuthorization: StateFlow<Boolean> = _needsAuthorization.asStateFlow()

    private val _current = MutableStateFlow<NavUpdate?>(null)
    /** Latest update, `null` when OsmAnd is unavailable or not navigating. */
    val current: StateFlow<NavUpdate?> = _current.asStateFlow()
    override val updates: Flow<NavUpdate?> = current

    fun start() {
        synchronized(lock) { started = true }
        bind()
    }

    fun stop() {
        val (oldApi, oldIds) = synchronized(lock) {
            started = false
            generation++
            val previous = api to registrations
            api = null
            registrations = emptyList()
            clear()
            previous
        }
        oldApi?.let { a -> oldIds.forEach { id -> runCatching { a.unregister(id) } } }
        runCatching { binder.unbind() }
    }

    private fun bind() {
        val bound = runCatching {
            binder.bind(onConnected = ::onConnected, onDisconnected = ::onDisconnected)
        }.getOrDefault(false)
        if (!bound) synchronized(lock) { clear() }
    }

    private fun onConnected(connected: OsmAndApi) {
        val gen = synchronized(lock) {
            if (!started) return
            api = connected
            ++generation
        }
        // IPC outside the lock: OsmAnd may deliver a callback on another thread before these calls return.
        val ids = listOf(
            runCatching {
                connected.registerNavigation { distanceTo, turnType ->
                    synchronized(lock) { if (gen == generation) _current.value = merged(reducer.onDirection(distanceTo, turnType)) }
                }
            }.getOrDefault(FAILED),
            runCatching {
                connected.registerVoice { played ->
                    synchronized(lock) { if (gen == generation) reducer.onVoicePlayed(played)?.let { _current.value = merged(it) } }
                }
            }.getOrDefault(FAILED),
        )
        synchronized(lock) {
            if (gen != generation) return
            registrations = ids.filter { it >= 0 }
            _needsAuthorization.value = REFUSED in ids
            _available.value = ids.all { it >= 0 }
            if (!_available.value) _current.value = null
        }
    }

    /** Called by the notification listener for OsmAnd's navigation notification. */
    fun onNotification(update: OsmAndNotificationParser.Extra) = synchronized(lock) {
        extra = update
        _current.value = merged(reducer.current)
    }

    /** OsmAnd sends no final callback when navigation stops; the removal of its navigation notification is the signal. */
    fun onNavigationNotificationRemoved() = synchronized(lock) {
        extra = OsmAndNotificationParser.Extra(null, null)
        _current.value = reducer.onNavigationStopped()
    }

    private fun merged(update: NavUpdate?): NavUpdate? = update?.copy(
        instruction = extra.instruction ?: update.instruction,
        eta = extra.eta ?: update.eta,
    )

    private fun onDisconnected(bindingDied: Boolean) {
        val rebind = synchronized(lock) {
            generation++
            api = null
            registrations = emptyList()
            clear()
            bindingDied && started
        }
        if (rebind) {
            // Android does not reconnect a dead binding (e.g. OsmAnd was updated): unbind and bind again.
            runCatching { binder.unbind() }
            bind()
        }
    }

    private companion object {
        /** OsmAnd's return value when the calling app is not allowed to use the method. */
        const val REFUSED = -1L
        /** Our marker for a register call that threw (OsmAnd died mid-call). */
        const val FAILED = -2L
    }

    private fun clear() {
        extra = OsmAndNotificationParser.Extra(null, null)
        _available.value = false
        _current.value = reducer.onNavigationStopped()
    }
}
