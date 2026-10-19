package org.openmyglass.core.nav

/**
 * Google Maps' ongoing navigation notification → [NavUpdate] (formats in docs/navigation-sources.md):
 * title = distance to the maneuver, text = road / "toward street" / a sentence, subText = "time · distance · ETA".
 * The maneuver itself is only in the icon, recognized through [MapsIcons].
 */
object MapsNotificationParser {
    private val TOWARD = Regex("""^(verso|toward)\s+""", RegexOption.IGNORE_CASE)
    private val ETA_LABELS = Regex("""\b(ETA|Arrivo|arrival)\b""", RegexOption.IGNORE_CASE)

    fun parse(n: NotificationText): NavUpdate? {
        val title = n.title?.trim().orEmpty()
        val text = n.text?.trim().orEmpty()
        val subText = n.subText?.trim().orEmpty()
        if (title.isEmpty() && text.isEmpty() && subText.isEmpty()) return null

        var street: String? = null
        var instruction: String? = null
        when {
            text.isEmpty() -> Unit
            TOWARD.containsMatchIn(text) -> street = text.replace(TOWARD, "")
            " / " in text -> street = text.substringAfterLast(" / ")
            isSentence(text) -> instruction = text
            else -> street = text
        }
        val distance = Distances.toMeters(title)
        if (distance == null && title.isNotEmpty() && instruction == null) instruction = title

        return NavUpdate(
            maneuver = MapsIcons.maneuverFor(n.iconSha256),
            distanceMeters = distance,
            instruction = instruction,
            street = street,
            eta = subText.split(" · ").lastOrNull()?.replace(ETA_LABELS, "")?.trim()?.takeIf { it.isNotEmpty() && subText.contains(" · ") },
            source = "gmaps",
            iconSha256 = n.iconSha256,
        )
    }

    /** Heuristic [unverified beyond the fixtures]: messages end with an ellipsis or have four or more words; street names rarely do. */
    private fun isSentence(text: String) =
        text.endsWith("...") || text.endsWith("…") || text.split(Regex("""\s+""")).size >= 4
}

/** Known Google Maps maneuver icons by SHA-256 of their pixels (built from captures; grows as icons get labeled). */
object MapsIcons {
    private val BY_HASH = mapOf(
        // Captured with text "Fai una inversione a U" (U-turn), gmaps-car-it-icons.jsonl
        "13b4feacacd89ee060f297a714a5ed9a09a929d50e7bb10f71e7689f195e96c1" to Maneuver.U_TURN_LEFT,
    )

    fun maneuverFor(sha256: String?): Maneuver = sha256?.let { BY_HASH[it.lowercase()] } ?: Maneuver.UNKNOWN
}
