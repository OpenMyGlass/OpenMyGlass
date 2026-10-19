package org.openmyglass.core.nav

/**
 * OsmAnd's navigation notification adds what its AIDL API lacks (docs/navigation-sources.md):
 * bigText line 1 = instruction with street + distance, line 2 = "remaining • time • ETA • speed".
 */
object OsmAndNotificationParser {
    data class Extra(val instruction: String?, val eta: String?)

    fun parse(n: NotificationText): Extra {
        val lines = n.bigText?.lines()?.map { it.trim() }?.filter { it.isNotEmpty() }.orEmpty()
        // Near the destination OsmAnd drops the instruction line and shows only the stats line.
        val stats = lines.lastOrNull { isStats(it) }
        val instruction = lines.firstOrNull { !isStats(it) }?.replace(Distances.TRAILING, "")?.trim()?.takeIf { it.isNotEmpty() }
        val eta = stats?.split(" • ")?.getOrNull(2)?.trim()?.takeIf { it.isNotEmpty() }
        return Extra(instruction, eta)
    }

    /** "remaining • time • ETA • speed": at least three bullet-separated parts. */
    private fun isStats(line: String) = line.split(" • ").size >= 3
}
