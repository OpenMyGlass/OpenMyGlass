package org.openmyglass.android

import android.app.Activity
import android.os.Bundle
import android.widget.LinearLayout
import android.widget.TextView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch
import org.openmyglass.android.nav.AndroidOsmAndBinder
import org.openmyglass.android.nav.OsmAndNavigationSource

/** Developer screen: navigation-source status. Grows as features land. */
class DebugActivity : Activity() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private lateinit var osmand: OsmAndNavigationSource

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val osmandStatus = TextView(this)
        setContentView(LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(32, 32, 32, 32)
            addView(TextView(this@DebugActivity).apply { text = "OpenMyGlass — debug" })
            addView(osmandStatus)
        })
        osmand = OsmAndNavigationSource(AndroidOsmAndBinder(applicationContext))
        scope.launch {
            osmand.available.combine(osmand.current) { available, update -> available to update }
                .collect { (available, update) ->
                    osmandStatus.text = "OsmAnd: ${if (available) "connected" else "unavailable"}\n" +
                        (update?.let { "${it.maneuver} in ${it.distanceMeters ?: "?"} m\n${it.instruction.orEmpty()}" }
                            ?: "no active navigation")
                }
        }
        osmand.start()
    }

    override fun onDestroy() {
        osmand.stop()
        scope.cancel()
        super.onDestroy()
    }
}
