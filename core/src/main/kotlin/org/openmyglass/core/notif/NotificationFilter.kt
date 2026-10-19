package org.openmyglass.core.notif

/**
 * Which notifications reach Glass and which may wake its screen (architecture §5.3, §6). Ongoing ones
 * (navigation, music, downloads, foreground services) and group summaries never do: they would flood a small
 * screen and the radio.
 */
class NotificationFilter(
    private val ownPackage: String,
    private val muted: Set<String> = emptySet(),
    private val wakeEnabled: Boolean = true,
    private val wakeCategories: Set<String> = setOf(
        NotificationItem.CATEGORY_MESSAGE, NotificationItem.CATEGORY_CALL,
        NotificationItem.CATEGORY_ALARM, NotificationItem.CATEGORY_REMINDER,
    ),
) {
    fun accept(item: NotificationItem): Boolean {
        if (item.ongoing || item.groupSummary || item.category == NotificationItem.CATEGORY_PROGRESS) return false
        if (item.packageName in muted) return false
        if (item.packageName == ownPackage && item.channelId != TEST_CHANNEL) return false
        if (item.title.isNullOrBlank() && item.text.isNullOrBlank()) return false
        val important = item.category == NotificationItem.CATEGORY_MESSAGE || item.category == NotificationItem.CATEGORY_CALL
        return important || item.importance >= NotificationItem.IMPORTANCE_DEFAULT
    }

    fun wake(item: NotificationItem): Boolean = wakeEnabled && item.category in wakeCategories

    companion object {
        /** Channel of the phone app's own test notifications (debug screen), let through for end-to-end tests. */
        const val TEST_CHANNEL = "glass-test"
    }
}
