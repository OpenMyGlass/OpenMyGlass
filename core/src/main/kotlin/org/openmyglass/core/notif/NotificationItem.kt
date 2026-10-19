package org.openmyglass.core.notif

import com.google.protobuf.ByteString
import org.openmyglass.protocol.v1.Bridge

data class ItemAction(val title: String, val reply: Boolean)

/** A phone notification as the relay sees it, before filtering. Platform-independent (Android maps into it). */
data class NotificationItem(
    val key: String,
    val packageName: String,
    val appName: String,
    val title: String?,
    val text: String?,
    val postedAt: Long,
    val category: String?,
    val ongoing: Boolean,
    val groupSummary: Boolean,
    /** Channel importance, Android's scale: [IMPORTANCE_LOW] … [IMPORTANCE_HIGH]. */
    val importance: Int,
    val channelId: String?,
    /** In the notification's own order: an action's position is its index for firing it. */
    val actions: List<ItemAction>,
    val iconSha256: String?,
) {
    /** What Glass gets: truncated so a frame stays far below the 64 KiB cap (BLE-friendly, architecture §10). */
    fun toProto(wake: Boolean): Bridge.Notification = Bridge.Notification.newBuilder().also { b ->
        b.key = key
        b.appName = appName.clip(APP_MAX)
        b.title = title.orEmpty().clip(TITLE_MAX)
        b.text = text.orEmpty().clip(TEXT_MAX)
        b.postedAtMs = postedAt
        b.wake = wake
        iconSha256?.let { b.iconSha256 = ByteString.copyFrom(it.chunked(2).map { h -> h.toInt(16).toByte() }.toByteArray()) }
        // Untitled actions cannot be shown; the index stays the action's position on the phone
        actions.withIndex().filter { it.value.title.isNotBlank() }.take(ACTIONS_MAX).forEach { (i, a) ->
            b.addActions(Bridge.NotificationActionInfo.newBuilder().setIndex(i).setTitle(a.title.clip(ACTION_TITLE_MAX)).setReply(a.reply))
        }
    }.build()

    companion object {
        const val TITLE_MAX = 200
        const val TEXT_MAX = 1_000
        const val APP_MAX = 60
        const val ACTION_TITLE_MAX = 40
        const val ACTIONS_MAX = 3

        // Android's values (NotificationManager.IMPORTANCE_*, Notification.CATEGORY_*)
        const val IMPORTANCE_LOW = 2
        const val IMPORTANCE_DEFAULT = 3
        const val IMPORTANCE_HIGH = 4
        const val CATEGORY_MESSAGE = "msg"
        const val CATEGORY_CALL = "call"
        const val CATEGORY_ALARM = "alarm"
        const val CATEGORY_REMINDER = "reminder"
        const val CATEGORY_PROGRESS = "progress"

        /** At most [max] code points (never splits a surrogate pair); an ellipsis marks the cut. */
        internal fun String.clip(max: Int): String {
            if (codePointCount(0, length) <= max) return this
            return substring(0, offsetByCodePoints(0, max - 1)) + "…"
        }
    }
}
