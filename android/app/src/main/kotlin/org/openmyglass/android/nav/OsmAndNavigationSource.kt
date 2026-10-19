package org.openmyglass.android.nav

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.openmyglass.core.nav.NavUpdate
import org.openmyglass.core.nav.NavUpdateReducer
import org.openmyglass.core.nav.NavigationSource

/**
 * Navigation updates from OsmAnd's public AIDL API (docs/navigation-sources.md).
 * AIDL callbacks arrive on binder threads, so state changes are serialized on [lock].
 */
class OsmAndNavigationSource(private val binder: OsmAndBinder) : NavigationSource {
    override val name = "osmand"

    private val lock = Any()
    private val reducer = NavUpdateReducer(name)
    private var api: OsmAndApi? = null
    private val registrations = mutableListOf<Long>()

    private val _available = MutableStateFlow(false)
    override val available: StateFlow<Boolean> = _available.asStateFlow()

    private val _current = MutableStateFlow<NavUpdate?>(null)
    /** Latest update, `null` when OsmAnd is unavailable or not navigating. */
    val current: StateFlow<NavUpdate?> = _current.asStateFlow()
    override val updates: Flow<NavUpdate?> = current

    fun start() {
        val bound = binder.bind(onConnected = ::onConnected, onDisconnected = ::onDisconnected)
        if (!bound) onDisconnected()
    }

    fun stop() = synchronized(lock) {
        api?.let { a -> registrations.forEach(a::unregister) }
        registrations.clear()
        api = null
        binder.unbind()
        clear()
    }

    private fun onConnected(connected: OsmAndApi) = synchronized(lock) {
        api = connected
        registrations.clear()
        registrations += connected.registerNavigation { distanceTo, turnType ->
            synchronized(lock) { _current.value = reducer.onDirection(distanceTo, turnType) }
        }
        registrations += connected.registerVoice { played ->
            synchronized(lock) { reducer.onVoicePlayed(played)?.let { _current.value = it } }
        }
        _available.value = true
    }

    private fun onDisconnected() = synchronized(lock) {
        api = null
        registrations.clear()
        clear()
    }

    private fun clear() {
        _available.value = false
        _current.value = reducer.onNavigationStopped()
    }
}
