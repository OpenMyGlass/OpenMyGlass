package org.openmyglass.android

import android.Manifest
import android.annotation.SuppressLint
import android.app.Activity
import android.bluetooth.BluetoothManager
import android.os.Build
import android.os.Bundle
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.RadioButton
import android.widget.RadioGroup
import android.widget.ScrollView
import android.widget.TextView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.openmyglass.android.nav.NavigationHub
import org.openmyglass.android.service.OpenMyGlassService
import org.openmyglass.android.transport.BridgeMode
import org.openmyglass.android.transport.Settings

/** Chooses the bonded Glass and the bridge transport, and starts or stops the connection service. */
class SettingsActivity : Activity() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private lateinit var settings: Settings
    private lateinit var devices: RadioGroup
    private var chosenAddress: String? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        settings = Settings(this)
        val current = settings.load()
        chosenAddress = current.glassAddress
        val status = TextView(this)
        devices = RadioGroup(this)
        val modes = RadioGroup(this).apply {
            BridgeMode.entries.forEach { mode ->
                addView(RadioButton(this@SettingsActivity).apply {
                    id = mode.ordinal + 1
                    text = when (mode) {
                        BridgeMode.BLUETOOTH -> "Bridge over Bluetooth (Glass)"
                        BridgeMode.TCP -> "Bridge over TCP (debug)"
                        BridgeMode.OFF -> "No bridge"
                    }
                })
            }
            check(current.bridgeMode.ordinal + 1)
        }
        val tcp = EditText(this).apply { setText("${current.tcpHost}:${current.tcpPort}"); isSingleLine = true }
        val companion = CheckBox(this).apply { text = "Companion link to the Glass firmware"; isChecked = current.companionEnabled }
        val content = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(32, 32, 32, 32)
            addView(TextView(this@SettingsActivity).apply { text = "OpenMyGlass — connection" })
            addView(status)
            addView(Button(this@SettingsActivity).apply {
                text = "Grant Bluetooth and notification permissions"
                setOnClickListener { requestPermissions() }
            })
            addView(TextView(this@SettingsActivity).apply { text = "Glass (paired devices):" })
            addView(devices)
            addView(modes)
            addView(tcp)
            addView(companion)
            addView(Button(this@SettingsActivity).apply {
                text = "Save and connect"
                setOnClickListener {
                    val (host, port) = tcp.text.toString().split(":").let { it[0] to (it.getOrNull(1)?.toIntOrNull() ?: 7825) }
                    settings.save(current.copy(
                        bridgeMode = BridgeMode.entries[modes.checkedRadioButtonId - 1],
                        glassAddress = chosenAddress, tcpHost = host, tcpPort = port,
                        companionEnabled = companion.isChecked,
                    ))
                    if (!OpenMyGlassService.start(this@SettingsActivity)) requestPermissions()
                }
            })
            addView(Button(this@SettingsActivity).apply {
                text = "Disconnect"
                setOnClickListener { OpenMyGlassService.stop(this@SettingsActivity) }
            })
        }
        setContentView(ScrollView(this).apply { fitsSystemWindows = true; addView(content) })
        scope.launch { NavigationHub.bridgeStatus.collect { status.text = it } }
    }

    override fun onResume() {
        super.onResume()
        listBondedDevices()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        listBondedDevices()
    }

    private fun requestPermissions() {
        val wanted = buildList {
            if (Build.VERSION.SDK_INT >= 31) add(Manifest.permission.BLUETOOTH_CONNECT)
            if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
        }
        if (wanted.isNotEmpty()) requestPermissions(wanted.toTypedArray(), 1)
    }

    @SuppressLint("MissingPermission")   // guarded by hasBluetoothPermission
    private fun listBondedDevices() {
        devices.removeAllViews()
        if (!OpenMyGlassService.hasBluetoothPermission(this)) {
            devices.addView(TextView(this).apply { text = "Bluetooth permission needed to list paired devices" })
            return
        }
        val bonded = getSystemService(BluetoothManager::class.java)?.adapter?.bondedDevices.orEmpty()
        if (bonded.isEmpty()) devices.addView(TextView(this).apply { text = "No paired devices: pair Glass in Bluetooth settings" })
        bonded.forEach { device ->
            devices.addView(RadioButton(this).apply {
                text = "${device.name ?: "?"} (${device.address})"
                isChecked = device.address == chosenAddress
                setOnClickListener { chosenAddress = device.address }
            })
        }
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }
}
