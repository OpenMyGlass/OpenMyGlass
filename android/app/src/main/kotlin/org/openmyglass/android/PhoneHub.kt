package org.openmyglass.android

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import org.openmyglass.android.nav.AndroidOsmAndBinder
import org.openmyglass.android.nav.MapsNavigationSource
import org.openmyglass.android.nav.OsmAndNavigationSource
import org.openmyglass.android.notifications.NotificationActions
import org.openmyglass.android.notifications.NotificationSettings
import org.openmyglass.core.bridge.NavigationRelay
import org.openmyglass.core.notif.NotificationFilter
import org.openmyglass.core.notif.NotificationItem
import org.openmyglass.core.notif.NotificationRelay
import java.util.Collections
import java.util.concurrent.ConcurrentHashMap

/**
 * Process-wide wiring of the phone side: navigation sources → navigation relay, notifications → notification
 * relay. Shared by the notification listener, the UI and OpenMyGlassService (which owns the bridge link).
 */
object PhoneHub {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var initialized = false

    lateinit var osmand: OsmAndNavigationSource
        private set
    val maps = MapsNavigationSource()

    /** Recent maneuver icons by pixel SHA-256 (hex) → PNG; small LRU, icons are a few KiB each. */
    private val icons: MutableMap<String, ByteArray> = Collections.synchronizedMap(
        object : LinkedHashMap<String, ByteArray>(32, 0.75f, true) {
            override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, ByteArray>) = size > 64
        }
    )
    lateinit var relay: NavigationRelay
        private set
    lateinit var notifications: NotificationRelay
        private set

    /** App icons for notifications by SHA-256 (hex) → 48 px PNG. */
    private val appIcons = ConcurrentHashMap<String, ByteArray>()
    /** Apps whose notifications were seen in this process (package → name), for the mute list in settings. */
    val seenApps: MutableMap<String, String> = ConcurrentHashMap()

    private val _bridgeStatus = MutableStateFlow("bridge: service not started")
    /** Link status for the UI, written by OpenMyGlassService. */
    val bridgeStatus: StateFlow<String> = _bridgeStatus.asStateFlow()

    fun setBridgeStatus(status: String) {
        _bridgeStatus.value = status
    }

    @Synchronized
    fun init(context: Context) {
        if (initialized) return
        osmand = OsmAndNavigationSource(AndroidOsmAndBinder(context.applicationContext)).also { it.start() }
        relay = NavigationRelay(listOf(osmand, maps), icons = { icons[it] }, scope = scope)
        val app = context.applicationContext
        notifications = NotificationRelay(
            filter = {
                val s = NotificationSettings(app).load()
                NotificationFilter(app.packageName, muted = s.muted, wakeEnabled = s.wake)
            },
            icons = { appIcons[it] },
            onAction = { key, request -> NotificationActions.perform(app, key, request) },
        )
        initialized = true
    }

    fun putIcon(sha256Hex: String, png: ByteArray) {
        icons[sha256Hex] = png
    }

    fun putAppIcon(sha256Hex: String, png: ByteArray) {
        appIcons[sha256Hex] = png
    }

    /** Listener events applied one at a time, in arrival order (a removal must not overtake its post). */
    private val notificationEvents = Channel<suspend () -> Unit>(Channel.UNLIMITED).also { events ->
        scope.launch { for (event in events) runCatching { event() } }
    }

    fun notificationPosted(item: NotificationItem) {
        seenApps[item.packageName] = item.appName
        notificationEvents.trySend { notifications.posted(item) }
    }

    /** The phone's active notifications, after the listener reconnected or settings changed. */
    fun notificationsSync(current: List<NotificationItem>) {
        current.forEach { seenApps[it.packageName] = it.appName }
        notificationEvents.trySend { notifications.sync(current) }
    }

    fun notificationsDisabled() {
        notificationEvents.trySend { notifications.clear() }
    }

    fun notificationRemoved(key: String) {
        notificationEvents.trySend { notifications.removed(key) }
    }
}
