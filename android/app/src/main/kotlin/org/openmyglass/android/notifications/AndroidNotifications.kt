package org.openmyglass.android.notifications

import android.app.Notification
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Canvas
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import org.openmyglass.android.PhoneHub
import java.io.ByteArrayOutputStream
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap

/** Reads a StatusBarNotification into [NotificationFields]; app icons are encoded once per package. */
object AndroidNotifications {
    private const val ICON_PX = 48
    private val iconByPackage = ConcurrentHashMap<String, String>()

    fun fields(context: Context, sbn: StatusBarNotification, ranking: NotificationListenerService.RankingMap?): NotificationFields {
        val n = sbn.notification
        val e = n.extras
        val pm = context.packageManager
        val appName = runCatching { pm.getApplicationLabel(pm.getApplicationInfo(sbn.packageName, 0)).toString() }
            .getOrDefault(sbn.packageName)
        val importance = ranking?.let { r ->
            NotificationListenerService.Ranking().takeIf { r.getRanking(sbn.key, it) }?.importance
        } ?: 3
        // MessagingStyle messages are Bundles with "text" and "sender" (the platform reader is API 30+)
        @Suppress("DEPRECATION")
        val last = e.getParcelableArray(Notification.EXTRA_MESSAGES)?.lastOrNull() as? android.os.Bundle
        val lastMessage = last?.getCharSequence("text")?.let { text ->
            listOfNotNull(last.getCharSequence("sender")?.toString(), text.toString()).joinToString(": ")
        }
        return NotificationFields(
            key = sbn.key,
            packageName = sbn.packageName,
            appName = appName,
            postTime = sbn.postTime,
            category = n.category,
            ongoing = sbn.isOngoing || (n.flags and Notification.FLAG_FOREGROUND_SERVICE) != 0,
            groupSummary = (n.flags and Notification.FLAG_GROUP_SUMMARY) != 0,
            importance = importance,
            channelId = n.channelId,
            title = e.getCharSequence(Notification.EXTRA_TITLE)?.toString(),
            conversationTitle = e.getCharSequence(Notification.EXTRA_CONVERSATION_TITLE)?.toString(),
            text = e.getCharSequence(Notification.EXTRA_TEXT)?.toString(),
            bigText = e.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString(),
            lastMessage = lastMessage,
            actions = n.actions.orEmpty().map { ActionFields(it.title?.toString(), !it.remoteInputs.isNullOrEmpty()) },
            iconSha256 = appIcon(pm, sbn.packageName),
        )
    }

    /** The app icon as a 48 px PNG, registered in [PhoneHub]; its SHA-256 (hex) or null. */
    private fun appIcon(pm: PackageManager, pkg: String): String? = iconByPackage[pkg] ?: runCatching {
        val drawable = pm.getApplicationIcon(pkg)
        val bitmap = Bitmap.createBitmap(ICON_PX, ICON_PX, Bitmap.Config.ARGB_8888)
        drawable.setBounds(0, 0, ICON_PX, ICON_PX)
        drawable.draw(Canvas(bitmap))
        val png = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        val hex = MessageDigest.getInstance("SHA-256").digest(png).joinToString("") { "%02x".format(it) }
        PhoneHub.putAppIcon(hex, png)
        iconByPackage[pkg] = hex
        hex
    }.getOrNull()
}
