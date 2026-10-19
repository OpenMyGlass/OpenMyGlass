package org.openmyglass.android.notifications

import org.openmyglass.core.notif.ItemAction
import org.openmyglass.core.notif.NotificationItem

/** The fields of a StatusBarNotification the relay needs, read by [AndroidNotifications] (kept plain for tests). */
data class NotificationFields(
    val key: String,
    val packageName: String,
    val appName: String,
    val postTime: Long,
    val category: String?,
    val ongoing: Boolean,
    val groupSummary: Boolean,
    val importance: Int,
    val channelId: String?,
    val title: String?,
    val conversationTitle: String?,
    val text: String?,
    val bigText: String?,
    /** MessagingStyle: the newest message, "Sender: text". */
    val lastMessage: String?,
    val actions: List<ActionFields>,
    val iconSha256: String?,
)

data class ActionFields(val title: String?, val hasRemoteInput: Boolean)

object NotificationMapper {
    fun map(f: NotificationFields) = NotificationItem(
        key = f.key,
        packageName = f.packageName,
        appName = f.appName,
        title = f.title?.takeIf { it.isNotBlank() } ?: f.conversationTitle?.takeIf { it.isNotBlank() },
        text = listOf(f.bigText, f.text, f.lastMessage).firstOrNull { !it.isNullOrBlank() },
        postedAt = f.postTime,
        category = f.category,
        ongoing = f.ongoing,
        groupSummary = f.groupSummary,
        importance = f.importance,
        channelId = f.channelId,
        // Positions matter (the index fires the action); Glass hides actions without a title
        actions = f.actions.map { ItemAction(it.title?.trim().orEmpty(), it.hasRemoteInput) },
        iconSha256 = f.iconSha256,
    )
}
