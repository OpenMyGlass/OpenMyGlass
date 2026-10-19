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
        runCatching { captureFile(this).appendText(line + "\n") }
    }

    /** One extra at a time: a value that cannot be read (e.g. a custom Parcelable) becomes a placeholder. */
    private fun Bundle.toMap(): Map<String, Any?> = keySet().associateWith { key ->
        runCatching { convert(@Suppress("DEPRECATION") get(key)) }
            .getOrElse { mapOf("type" to "unreadable", "error" to it::class.java.simpleName) }
    }

    private fun convert(v: Any?): Any? = when (v) {
        is Bitmap -> imageMeta(v)
        is Icon -> iconMeta(v)
        is Bundle -> v.toMap()
        is Array<*> -> v.map(::convert)
        is List<*> -> v.map(::convert)
        else -> v
    }

    /** Only metadata and a hash leave the device: icons from other apps are never stored as images. */
    private fun imageMeta(bitmap: Bitmap): Map<String, Any?> {
        val meta = mapOf("type" to "image", "width" to bitmap.width, "height" to bitmap.height)
        val sha = runCatching {
            val readable = if (bitmap.config == Bitmap.Config.HARDWARE) bitmap.copy(Bitmap.Config.ARGB_8888, false) else bitmap
            val buffer = ByteBuffer.allocate(readable.byteCount).also { readable.copyPixelsToBuffer(it) }
            MessageDigest.getInstance("SHA-256").digest(buffer.array()).joinToString("") { "%02x".format(it) }
        }.getOrNull()
        return if (sha != null) meta + ("sha256" to sha) else meta
    }

    /** Bitmap icons (e.g. Google Maps' maneuver arrows) get the same hash as bitmaps, so they can be recognized. */
    private fun iconMeta(icon: Icon): Map<String, Any?> {
        val drawable = runCatching { icon.loadDrawable(this) }.getOrNull()
        val bitmap = (drawable as? android.graphics.drawable.BitmapDrawable)?.bitmap
        return mapOf("type" to "icon", "iconType" to icon.type) + (bitmap?.let { imageMeta(it) - "type" } ?: emptyMap())
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
        fun captureFile(context: android.content.Context): File {
            val dir = context.getExternalFilesDir("captures") ?: File(context.filesDir, "captures").apply { mkdirs() }
            return File(dir, "navigation-notifications.jsonl")
        }
    }
}

