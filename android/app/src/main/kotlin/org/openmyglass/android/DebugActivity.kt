package org.openmyglass.android

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.provider.Settings
import android.widget.Button
import android.widget.CheckBox
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
import org.openmyglass.android.notifications.OpenMyGlassNotificationListener

/** Developer screen: navigation-source status. Grows as features land. */
class DebugActivity : Activity() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private lateinit var osmand: OsmAndNavigationSource

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val osmandStatus = TextView(this)
        val allowOsmAnd = Button(this).apply {
            text = "Allow OpenMyGlass in OsmAnd"
            visibility = android.view.View.GONE
            setOnClickListener {
                AndroidOsmAndBinder.permissionRequestIntent(this@DebugActivity)
                    ?.let { startActivityForResult(it, REQUEST_OSMAND_PERMISSION) }
            }
        }
        setContentView(LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            // targetSdk 35+ enforces edge-to-edge: keep content clear of the status and app bars
            fitsSystemWindows = true
            setPadding(32, 32, 32, 32)
            addView(TextView(this@DebugActivity).apply { text = "OpenMyGlass — debug" })
            addView(osmandStatus)
            addView(allowOsmAnd)
            addView(Button(this@DebugActivity).apply {
                text = "Notification access settings"
                setOnClickListener { startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)) }
            })
            addView(CheckBox(this@DebugActivity).apply {
                text = "Capture navigation notifications (Maps, OsmAnd)"
                isChecked = OpenMyGlassNotificationListener.isCaptureEnabled(this@DebugActivity)
                setOnCheckedChangeListener { _, checked ->
                    OpenMyGlassNotificationListener.setCaptureEnabled(this@DebugActivity, checked)
                }
            })
            addView(TextView(this@DebugActivity).apply {
                text = "Captures: " + OpenMyGlassNotificationListener.captureFile(this@DebugActivity).absolutePath
            })
        })
        osmand = OsmAndNavigationSource(AndroidOsmAndBinder(applicationContext))
        scope.launch {
            combine(osmand.available, osmand.needsAuthorization, osmand.current) { a, n, u -> Triple(a, n, u) }
                .collect { (available, needsAuth, update) ->
                    val status = when {
                        available -> "connected"
                        needsAuth -> "not authorized — in OsmAnd open Menu → Plugins and tap OpenMyGlass (OsmAnd ≥ 5.5: use the button below)"
                        else -> "unavailable"
                    }
                    allowOsmAnd.visibility = if (needsAuth) android.view.View.VISIBLE else android.view.View.GONE
                    osmandStatus.text = "OsmAnd: $status\n" +
                        (update?.let { "${it.maneuver} in ${it.distanceMeters ?: "?"} m\n${it.instruction.orEmpty()}" }
                            ?: "no active navigation")
                }
        }
        osmand.start()
    }

    @Deprecated("Activity result API is enough for a debug screen")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQUEST_OSMAND_PERMISSION && resultCode == RESULT_OK) {
            osmand.stop()
            osmand.start()
        }
    }

    private companion object {
        const val REQUEST_OSMAND_PERMISSION = 1
    }

    override fun onDestroy() {
        osmand.stop()
        scope.cancel()
        super.onDestroy()
    }
}
