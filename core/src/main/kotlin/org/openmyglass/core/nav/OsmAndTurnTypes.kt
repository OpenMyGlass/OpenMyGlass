package org.openmyglass.core.nav

/**
 * OsmAnd's `ADirectionInfo.turnType` values, from
 * `OsmAnd-java/src/main/java/net/osmand/router/TurnType.java` (osmandapp/OsmAnd).
 */
object OsmAndTurnTypes {
    fun toManeuver(turnType: Int): Maneuver = when (turnType) {
        1 -> Maneuver.STRAIGHT
        2 -> Maneuver.LEFT
        3 -> Maneuver.SLIGHT_LEFT
        4 -> Maneuver.SHARP_LEFT
        5 -> Maneuver.RIGHT
        6 -> Maneuver.SLIGHT_RIGHT
        7 -> Maneuver.SHARP_RIGHT
        8 -> Maneuver.KEEP_LEFT
        9 -> Maneuver.KEEP_RIGHT
        10 -> Maneuver.U_TURN_LEFT
        11 -> Maneuver.U_TURN_RIGHT
        12 -> Maneuver.OFF_ROUTE
        13 -> Maneuver.ROUNDABOUT
        14 -> Maneuver.ROUNDABOUT_LEFT
        else -> Maneuver.UNKNOWN
    }
}
