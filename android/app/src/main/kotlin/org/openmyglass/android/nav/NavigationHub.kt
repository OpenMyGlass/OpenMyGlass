package org.openmyglass.android.nav

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.openmyglass.core.bridge.NavigationRelay
import java.util.Collections

/**
 * Process-wide wiring of navigation sources → relay, shared by the notification listener, the UI and
 * OpenMyGlassService (which owns the bridge link).
 */
object NavigationHub {
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
        initialized = true
    }

    fun putIcon(sha256Hex: String, png: ByteArray) {
        icons[sha256Hex] = png
    }
}
