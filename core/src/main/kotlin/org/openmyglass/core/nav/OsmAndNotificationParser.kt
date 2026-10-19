package org.openmyglass.core.nav

/**
 * OsmAnd's navigation notification adds what its AIDL API lacks (docs/navigation-sources.md):
 * bigText line 1 = instruction with street + distance, line 2 = "remaining • time • ETA • speed".
 */
object OsmAndNotificationParser {
    data class Extra(val instruction: String?, val eta: String?)

    fun parse(n: NotificationText): Extra {
        val lines = n.bigText?.lines()?.map { it.trim() }.orEmpty()
        val instruction = lines.getOrNull(0)?.replace(Distances.TRAILING, "")?.trim()?.takeIf { it.isNotEmpty() }
        val eta = lines.getOrNull(1)?.split(" • ")?.getOrNull(2)?.trim()?.takeIf { it.isNotEmpty() }
        return Extra(instruction, eta)
    }
}
