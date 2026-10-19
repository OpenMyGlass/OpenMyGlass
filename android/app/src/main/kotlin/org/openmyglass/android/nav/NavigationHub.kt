package org.openmyglass.android.nav

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.openmyglass.core.bridge.BridgeLink
import org.openmyglass.core.bridge.NavigationRelay
import org.openmyglass.core.transport.TcpTransport
import java.util.Collections

/**
 * Process-wide wiring of navigation sources → relay → bridge link, shared by the notification listener and
 * the UI. The foreground service and the Bluetooth bridge transport replace the debug TCP connect in plan 4d.
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
    private lateinit var relay: NavigationRelay

    private val _bridgeStatus = MutableStateFlow("bridge: disconnected")
    val bridgeStatus: StateFlow<String> = _bridgeStatus.asStateFlow()
    // Only the UI thread calls connectBridge; @Volatile because the session coroutine reads currentLink.
    @Volatile private var bridgeJob: Job? = null
    @Volatile private var currentLink: BridgeLink? = null

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

    /** Debug: one bridge session over TCP to the bridge app or `sim-glass --bridge`. */
    fun connectBridge(host: String, port: Int, appVersion: String) {
        // Closing the old link unblocks its read so its cleanup runs now; its detach only ends its own session.
        currentLink?.close()
        bridgeJob?.cancel()
        bridgeJob = scope.launch {
            _bridgeStatus.value = "bridge: connecting to $host:$port"
            val link = try {
                val channel = withContext(Dispatchers.IO) { TcpTransport(host, port).connect() }
                BridgeLink(channel, appVersion).also { it.handshake(); currentLink = it }
            } catch (e: Exception) {
                _bridgeStatus.value = "bridge: failed (${e.message})"
                return@launch
            }
            var frames = 0
            val session = relay.attach { frame ->
                link.send(frame)
                frames++
                _bridgeStatus.value = "bridge: connected, $frames frames sent (last: ${frame.bodyCase})"
            }
            _bridgeStatus.value = "bridge: connected"
            try {
                link.incoming().collect { if (it.hasIconRequest()) relay.onIconRequest(it.iconRequest.sha256.toByteArray()) }
                _bridgeStatus.value = "bridge: disconnected"
            } catch (e: Exception) {
                _bridgeStatus.value = "bridge: lost (${e.message})"
            } finally {
                withContext(NonCancellable) { relay.detach(session) }
                link.close()
                if (currentLink === link) currentLink = null
            }
        }
    }
}
