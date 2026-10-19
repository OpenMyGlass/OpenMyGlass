package org.openmyglass.core.nav

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow

/** The next maneuver, independent of the navigation app that reported it. */
enum class Maneuver {
    STRAIGHT, LEFT, SLIGHT_LEFT, SHARP_LEFT, RIGHT, SLIGHT_RIGHT, SHARP_RIGHT, KEEP_LEFT, KEEP_RIGHT,
    U_TURN_LEFT, U_TURN_RIGHT, OFF_ROUTE, ROUNDABOUT, ROUNDABOUT_LEFT, ARRIVE, UNKNOWN,
}

/** What the navigation card on Glass shows. Fields a source does not provide are `null`. */
data class NavUpdate(
    val maneuver: Maneuver,
    val distanceMeters: Int?,
    val instruction: String?,
    val street: String?,
    val eta: String?,
    /** `"osmand"` or `"gmaps"`. */
    val source: String,
)

/** A phone-side source of navigation updates. [updates] emits `null` when no navigation is active. */
interface NavigationSource {
    val name: String
    val updates: Flow<NavUpdate?>
    val available: StateFlow<Boolean>
}
