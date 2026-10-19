package org.openmyglass.android.nav

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.openmyglass.core.nav.MapsNotificationParser
import org.openmyglass.core.nav.NavUpdate
import org.openmyglass.core.nav.NavigationSource
import org.openmyglass.core.nav.NotificationText

/** Fallback source: Google Maps' navigation notification, fed by the notification listener. */
class MapsNavigationSource : NavigationSource {
    override val name = "gmaps"
    private val _current = MutableStateFlow<NavUpdate?>(null)
    val current: StateFlow<NavUpdate?> = _current.asStateFlow()
    override val updates: Flow<NavUpdate?> = current
    /** Maps has no availability handshake: it is "available" while it posts navigation. */
    override val available: StateFlow<Boolean> = MutableStateFlow(true)

    fun onPosted(text: NotificationText) {
        _current.value = MapsNotificationParser.parse(text)
    }

    fun onRemoved() {
        _current.value = null
    }
}
