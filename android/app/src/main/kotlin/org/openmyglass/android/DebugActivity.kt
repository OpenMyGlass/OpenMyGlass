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
        val bridgeStatus = TextView(this)
        // The last TCP bridge address used (default: the host machine as seen from the emulator)
        val saved = org.openmyglass.android.transport.Settings(this).load()
        val bridgeAddress = android.widget.EditText(this).apply { setText("${saved.tcpHost}:${saved.tcpPort}"); isSingleLine = true }
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
            addView(bridgeStatus)
            addView(bridgeAddress)
            addView(Button(this@DebugActivity).apply {
                text = "Preset: Glass app on this device (127.0.0.1:7825)"
                setOnClickListener { bridgeAddress.setText("127.0.0.1:7825") }
            })
            addView(Button(this@DebugActivity).apply {
                text = "Preset: sim-glass on the host (10.0.2.2:7824)"
                setOnClickListener { bridgeAddress.setText("10.0.2.2:7824") }
            })
            addView(Button(this@DebugActivity).apply {
                text = "Connect bridge (debug TCP)"
                setOnClickListener {
                    val (host, port) = bridgeAddress.text.toString().split(":").let { it[0] to (it.getOrNull(1)?.toIntOrNull() ?: 7824) }
                    val links = org.openmyglass.android.transport.Settings(this@DebugActivity)
                    links.save(links.load().copy(bridgeMode = org.openmyglass.android.transport.BridgeMode.TCP, tcpHost = host, tcpPort = port))
                    if (!org.openmyglass.android.service.OpenMyGlassService.start(this@DebugActivity)) {
                        startActivity(Intent(this@DebugActivity, SettingsActivity::class.java))
                    }
                }
            })
            addView(Button(this@DebugActivity).apply {
                text = "Post test notification"
                setOnClickListener { org.openmyglass.android.notifications.TestNotification.post(this@DebugActivity) }
            })
            addView(Button(this@DebugActivity).apply {
                text = "Connection settings"
                setOnClickListener { startActivity(Intent(this@DebugActivity, SettingsActivity::class.java)) }
            })
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
        PhoneHub.init(applicationContext)
        osmand = PhoneHub.osmand
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
        scope.launch { PhoneHub.bridgeStatus.collect { bridgeStatus.text = it } }
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
        scope.cancel()
        super.onDestroy()
    }
}
