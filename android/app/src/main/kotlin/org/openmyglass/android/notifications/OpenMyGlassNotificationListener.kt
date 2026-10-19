package org.openmyglass.android.notifications

import android.app.Notification
import android.graphics.Bitmap
import android.graphics.drawable.Icon
import android.os.Bundle
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import org.openmyglass.android.nav.IconEncoder
import org.openmyglass.android.nav.NavigationHub
import org.openmyglass.core.nav.NotificationText
import org.openmyglass.core.nav.OsmAndNotificationParser
import java.io.File

/**
 * Notification access for OpenMyGlass. For now it only has a capture mode that records navigation
 * notifications (Google Maps, OsmAnd) as JSON lines, to learn which fields they carry.
 */
class OpenMyGlassNotificationListener : NotificationListenerService() {

    override fun onListenerConnected() {
        NavigationHub.init(applicationContext)
    }

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        if (sbn.packageName !in CAPTURED_PACKAGES) return
        NavigationHub.init(applicationContext)
        runCatching { feedNavigation(sbn) }
        if (!isCaptureEnabled(this)) return
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

    override fun onNotificationRemoved(sbn: StatusBarNotification) {
        if (sbn.packageName !in CAPTURED_PACKAGES || sbn.notification.category != Notification.CATEGORY_NAVIGATION) return
        NavigationHub.init(applicationContext)
        if (sbn.packageName == MAPS) NavigationHub.maps.onRemoved() else NavigationHub.osmand.onNavigationNotificationRemoved()
    }

    /** Navigation notifications → sources: Google Maps fully, OsmAnd for the street/ETA its AIDL API lacks. */
    private fun feedNavigation(sbn: StatusBarNotification) {
        val n = sbn.notification
        if (n.category != Notification.CATEGORY_NAVIGATION) return
        val e = n.extras
        val icon = (n.getLargeIcon()?.let { runCatching { it.loadDrawable(this) }.getOrNull() } as? android.graphics.drawable.BitmapDrawable)?.bitmap
        val encoded = icon?.let { IconEncoder.encode(it) }
        encoded?.let { (sha, png) -> NavigationHub.putIcon(sha, png) }
        val text = NotificationText(
            title = e.getCharSequence(Notification.EXTRA_TITLE)?.toString(),
            text = e.getCharSequence(Notification.EXTRA_TEXT)?.toString(),
            subText = e.getCharSequence(Notification.EXTRA_SUB_TEXT)?.toString(),
            bigText = e.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString(),
            iconSha256 = encoded?.first,
        )
        if (sbn.packageName == MAPS) NavigationHub.maps.onPosted(text)
        else NavigationHub.osmand.onNotification(OsmAndNotificationParser.parse(text))
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
        val sha = runCatching { IconEncoder.pixelSha256(bitmap) }.getOrNull()
        return if (sha != null) meta + ("sha256" to sha) else meta
    }

    /** Bitmap icons (e.g. Google Maps' maneuver arrows) get the same hash as bitmaps, so they can be recognized. */
    private fun iconMeta(icon: Icon): Map<String, Any?> {
        val drawable = runCatching { icon.loadDrawable(this) }.getOrNull()
        val bitmap = (drawable as? android.graphics.drawable.BitmapDrawable)?.bitmap
        return mapOf("type" to "icon", "iconType" to icon.type) + (bitmap?.let { imageMeta(it) - "type" } ?: emptyMap())
    }

    companion object {
        private const val MAPS = "com.google.android.apps.maps"
        val CAPTURED_PACKAGES = setOf(MAPS, "net.osmand.plus", "net.osmand")
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

