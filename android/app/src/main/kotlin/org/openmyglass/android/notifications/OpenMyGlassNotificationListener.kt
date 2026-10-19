package org.openmyglass.android.notifications

import android.graphics.Bitmap
import android.graphics.drawable.Icon
import android.os.Bundle
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import java.io.File
import java.nio.ByteBuffer
import java.security.MessageDigest

/**
 * Notification access for OpenMyGlass. For now it only has a capture mode that records navigation
 * notifications (Google Maps, OsmAnd) as JSON lines, to learn which fields they carry.
 */
class OpenMyGlassNotificationListener : NotificationListenerService() {

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        if (!isCaptureEnabled(this) || sbn.packageName !in CAPTURED_PACKAGES) return
        val n = sbn.notification
        @Suppress("DEPRECATION")
        val line = NotificationDump.toJson(
            NotificationDump.dump(
                packageName = sbn.packageName,
                postTime = sbn.postTime,
                category = n.category,
                isOngoing = sbn.isOngoing,
                extras = n.extras.toMap(),
                actionTitles = n.actions.orEmpty().map { it.title?.toString().orEmpty() },
                hasCustomContentView = n.contentView != null,
                hasCustomBigView = n.bigContentView != null,
            )
        )
        captureFile(this).appendText(line + "\n")
    }

    private fun Bundle.toMap(): Map<String, Any?> = keySet().associateWith { key ->
        @Suppress("DEPRECATION")
        when (val v = get(key)) {
            is Bitmap -> imageMeta(v)
            is Icon -> mapOf("type" to "icon", "iconType" to v.type)
            is Bundle -> v.toMap()
            is Array<*> -> v.map { if (it is Bundle) it.toMap() else it }
            else -> v
        }
    }

    private fun imageMeta(bitmap: Bitmap): Map<String, Any?> {
        val buffer = ByteBuffer.allocate(bitmap.byteCount).also { bitmap.copyPixelsToBuffer(it) }
        val sha = MessageDigest.getInstance("SHA-256").digest(buffer.array()).joinToString("") { "%02x".format(it) }
        return mapOf("type" to "image", "width" to bitmap.width, "height" to bitmap.height, "sha256" to sha)
    }

    companion object {
        val CAPTURED_PACKAGES = setOf("com.google.android.apps.maps", "net.osmand.plus", "net.osmand")
        private const val PREFS = "capture"
        private const val KEY_ENABLED = "enabled"

        fun isCaptureEnabled(context: android.content.Context) =
            context.getSharedPreferences(PREFS, MODE_PRIVATE).getBoolean(KEY_ENABLED, false)

        fun setCaptureEnabled(context: android.content.Context, enabled: Boolean) =
            context.getSharedPreferences(PREFS, MODE_PRIVATE).edit().putBoolean(KEY_ENABLED, enabled).apply()

        /** `adb pull /sdcard/Android/data/org.openmyglass.android/files/captures/` */
        fun captureFile(context: android.content.Context): File =
            File(context.getExternalFilesDir("captures"), "navigation-notifications.jsonl")
    }
}

