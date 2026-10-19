package org.openmyglass.android.notifications

import android.app.Notification
import android.graphics.Bitmap
import android.graphics.drawable.Icon
import android.os.Bundle
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import org.openmyglass.android.nav.IconEncoder
import org.openmyglass.android.PhoneHub
import org.openmyglass.core.nav.NotificationText
import org.openmyglass.core.nav.OsmAndNotificationParser
import java.io.File

/**
 * Notification access for OpenMyGlass: feeds the navigation sources (Google Maps, OsmAnd notifications) and the
 * notification relay to Glass, and carries out actions chosen on Glass. Capture mode (debug, opt-in) records
 * navigation notifications as JSON lines.
 */
class OpenMyGlassNotificationListener : NotificationListenerService() {

    override fun onListenerConnected() {
        instance = this
        PhoneHub.init(applicationContext)
        // The shade when access starts or resumes: removals may have been missed while unbound
        resync()
    }

    override fun onListenerDisconnected() {
        if (instance === this) instance = null
    }

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        PhoneHub.init(applicationContext)
        feedNotifications(sbn)
        if (sbn.packageName !in CAPTURED_PACKAGES) return
        PhoneHub.init(applicationContext)
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
        PhoneHub.init(applicationContext)
        PhoneHub.notificationRemoved(sbn.key)
        if (sbn.packageName !in CAPTURED_PACKAGES || sbn.notification.category != Notification.CATEGORY_NAVIGATION) return
        PhoneHub.init(applicationContext)
        if (sbn.packageName == MAPS) PhoneHub.maps.onRemoved() else PhoneHub.osmand.onNavigationNotificationRemoved()
    }

    /** Sends the whole active set through the relay (reconnect, settings change). */
    fun resync() {
        if (!NotificationSettings(this).load().enabled) return
        val ranking = runCatching { currentRanking }.getOrNull()
        val items = runCatching { activeNotifications }.getOrNull().orEmpty().mapNotNull { sbn ->
            runCatching { NotificationMapper.map(AndroidNotifications.fields(this, sbn, ranking)) }.getOrNull()
        }
        PhoneHub.notificationsSync(items)
    }

    private fun feedNotifications(sbn: StatusBarNotification) {
        if (!NotificationSettings(this).load().enabled) return
        runCatching { AndroidNotifications.fields(this, sbn, runCatching { currentRanking }.getOrNull()) }
            .onSuccess { PhoneHub.notificationPosted(NotificationMapper.map(it)) }
    }

    /** Navigation notifications → sources: Google Maps fully, OsmAnd for the street/ETA its AIDL API lacks. */
    private fun feedNavigation(sbn: StatusBarNotification) {
        val n = sbn.notification
        if (n.category != Notification.CATEGORY_NAVIGATION) return
        val e = n.extras
        val icon = (n.getLargeIcon()?.let { runCatching { it.loadDrawable(this) }.getOrNull() } as? android.graphics.drawable.BitmapDrawable)?.bitmap
        val encoded = icon?.let { IconEncoder.encode(it) }
        encoded?.let { (sha, png) -> PhoneHub.putIcon(sha, png) }
        val text = NotificationText(
            title = e.getCharSequence(Notification.EXTRA_TITLE)?.toString(),
            text = e.getCharSequence(Notification.EXTRA_TEXT)?.toString(),
            subText = e.getCharSequence(Notification.EXTRA_SUB_TEXT)?.toString(),
            bigText = e.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString(),
            iconSha256 = encoded?.first,
        )
        if (sbn.packageName == MAPS) PhoneHub.maps.onPosted(text)
        else PhoneHub.osmand.onNotification(OsmAndNotificationParser.parse(text))
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
        /** The connected listener, for actions chosen on Glass (dismiss, fire an action). */
        @Volatile var instance: OpenMyGlassNotificationListener? = null
            private set

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

