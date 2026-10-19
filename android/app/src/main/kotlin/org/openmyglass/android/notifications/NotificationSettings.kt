package org.openmyglass.android.notifications

import android.content.Context

/** Owner choices for notifications on Glass (architecture §5.3, §6). */
data class NotificationPrefs(
    val enabled: Boolean = true,
    /** Wake the Glass screen for new messages, calls, alarms and reminders. */
    val wake: Boolean = true,
    val muted: Set<String> = emptySet(),
)

class NotificationSettings(context: Context) {
    private val prefs = context.getSharedPreferences("notifications", Context.MODE_PRIVATE)

    fun load() = NotificationPrefs(
        enabled = prefs.getBoolean(ENABLED, true),
        wake = prefs.getBoolean(WAKE, true),
        muted = prefs.getStringSet(MUTED, emptySet()).orEmpty().toSet(),
    )

    fun save(p: NotificationPrefs) = prefs.edit()
        .putBoolean(ENABLED, p.enabled).putBoolean(WAKE, p.wake).putStringSet(MUTED, p.muted.toSet())
        .apply()

    private companion object {
        const val ENABLED = "enabled"
        const val WAKE = "wake"
        const val MUTED = "muted"
    }
}
