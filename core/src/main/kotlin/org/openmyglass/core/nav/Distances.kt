package org.openmyglass.core.nav

import kotlin.math.roundToInt

/** Locale-tolerant distance text ("200 m", "2,8 km", "700 ft", "0.3 mi") → meters. */
object Distances {
    private val PATTERN = Regex("""^(\d+(?:[.,]\d+)?)[\s ]*(km|m|mi|ft|yd)$""", RegexOption.IGNORE_CASE)
    /** The same distance at the end of a longer text, e.g. "Turn right and go Obergass 60 m". */
    val TRAILING = Regex("""[\s ]+\d+(?:[.,]\d+)?[\s ]*(km|m|mi|ft|yd)$""", RegexOption.IGNORE_CASE)

    private val METERS_PER_UNIT = mapOf("m" to 1.0, "km" to 1000.0, "mi" to 1609.344, "ft" to 0.3048, "yd" to 0.9144)

    fun toMeters(text: String): Int? {
        val match = PATTERN.matchEntire(text.trim()) ?: return null
        val value = match.groupValues[1].replace(',', '.').toDoubleOrNull() ?: return null
        return (value * METERS_PER_UNIT.getValue(match.groupValues[2].lowercase())).roundToInt()
    }
}
