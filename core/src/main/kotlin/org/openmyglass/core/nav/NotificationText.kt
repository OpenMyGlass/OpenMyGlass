package org.openmyglass.core.nav

/** The notification fields the navigation parsers read, already extracted from the Android notification. */
data class NotificationText(
    val title: String?,
    val text: String?,
    val subText: String?,
    val bigText: String?,
    /** SHA-256 (hex) of the notification's large icon, the maneuver arrow in Google Maps. */
    val iconSha256: String?,
)
