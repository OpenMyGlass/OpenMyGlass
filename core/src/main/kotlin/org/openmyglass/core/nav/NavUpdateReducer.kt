package org.openmyglass.core.nav

/**
 * Merges a navigation app's direction callbacks (maneuver + distance) and voice prompts (instruction text)
 * into [NavUpdate]s. Not thread-safe: call it from one thread or dispatcher.
 */
class NavUpdateReducer(private val source: String) {
    private var instruction: String? = null

    var current: NavUpdate? = null
        private set

    fun onDirection(distanceTo: Int, turnType: Int): NavUpdate =
        NavUpdate(
            maneuver = OsmAndTurnTypes.toManeuver(turnType),
            distanceMeters = distanceTo.takeIf { it >= 0 },
            instruction = instruction,
            street = null,
            eta = null,
            source = source,
        ).also { current = it }

    fun onVoicePlayed(played: List<String>): NavUpdate? {
        val text = played.lastOrNull { it.isNotBlank() } ?: return current
        instruction = text
        return current?.copy(instruction = text)?.also { current = it }
    }

    fun onNavigationStopped(): NavUpdate? {
        instruction = null
        current = null
        return null
    }
}
