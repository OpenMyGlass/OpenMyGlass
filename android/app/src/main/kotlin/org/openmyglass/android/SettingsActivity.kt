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
        PhoneHub.init(applicationContext)   // also opened from the service notification
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
            addNotificationSettings(this)
            addSpeechSettings(this)
            addMediaSettings(this)
        }
        setContentView(ScrollView(this).apply { fitsSystemWindows = true; addView(content) })
        scope.launch { PhoneHub.bridgeStatus.collect { status.text = it } }
    }

    /** Notifications on Glass: on/off, wake, and a mute list of the apps seen since the app started. */
    private fun addNotificationSettings(content: LinearLayout) {
        val store = org.openmyglass.android.notifications.NotificationSettings(this)
        val prefs = store.load()
        content.addView(TextView(this).apply { text = "Notifications on Glass:" })
        content.addView(CheckBox(this).apply {
            text = "Show phone notifications on Glass"
            isChecked = prefs.enabled
            setOnCheckedChangeListener { _, on ->
                store.save(store.load().copy(enabled = on))
                if (on) org.openmyglass.android.notifications.OpenMyGlassNotificationListener.instance?.resync()
                else PhoneHub.notificationsDisabled()
            }
        })
        content.addView(CheckBox(this).apply {
            text = "Wake Glass for new messages, calls, alarms and reminders"
            isChecked = prefs.wake
            setOnCheckedChangeListener { _, on -> store.save(store.load().copy(wake = on)) }
        })
        content.addView(TextView(this).apply { text = "Mute apps:" })
        PhoneHub.seenApps.entries.sortedBy { it.value.lowercase() }.forEach { (pkg, name) ->
            content.addView(CheckBox(this).apply {
                text = name
                isChecked = pkg in prefs.muted
                setOnCheckedChangeListener { _, muted ->
                    val now = store.load()
                    store.save(now.copy(muted = if (muted) now.muted + pkg else now.muted - pkg))
                    // Apply to what Glass shows now, not only to the next notifications
                    org.openmyglass.android.notifications.OpenMyGlassNotificationListener.instance?.resync()
                }
            })
        }
    }

    /** Dictated replies: language and the one-time model download (the only network use of dictation). */
    private fun addSpeechSettings(content: LinearLayout) {
        val speech = org.openmyglass.android.speech.SpeechSettings(this)
        val store = org.openmyglass.android.speech.ModelStore(this)
        val status = TextView(this)
        fun refresh() {
            val m = org.openmyglass.android.speech.ModelCatalog.forLanguage(speech.language)
            status.text = if (store.isInstalled(m)) "Speech model ${m.name}: installed (offline)"
            else "Speech model ${m.name}: not downloaded (${m.sizeMb} MB)"
        }
        content.addView(TextView(this).apply { text = "Dictated replies (transcribed offline on this phone):" })
        content.addView(RadioGroup(this).apply {
            listOf("it" to "Italiano", "en" to "English").forEachIndexed { i, (code, label) ->
                addView(RadioButton(this@SettingsActivity).apply {
                    id = 100 + i
                    text = label
                    isChecked = speech.language == code
                    setOnClickListener { speech.language = code; refresh() }
                })
            }
        })
        content.addView(status)
        content.addView(Button(this).apply {
            text = "Download speech model"
            setOnClickListener {
                val m = org.openmyglass.android.speech.ModelCatalog.forLanguage(speech.language)
                isEnabled = false
                scope.launch {
                    val result = kotlinx.coroutines.withContext(Dispatchers.IO) {
                        runCatching {
                            store.download(m) { done, total ->
                                if (total > 0) runOnUiThread { status.text = "Downloading ${m.name}: ${done * 100 / total}%" }
                            }
                        }
                    }
                    isEnabled = true
                    refresh()
                    result.exceptionOrNull()?.let { status.text = "Download failed: ${it.message}" }
                }
            }
        })
        refresh()
    }

    /** Photos and videos: automatic sync while Glass charges on Wi-Fi, or now. */
    private fun addMediaSettings(content: LinearLayout) {
        val settings = org.openmyglass.android.media.MediaSettings(this)
        val status = TextView(this)
        content.addView(TextView(this).apply { text = "Photos and videos from Glass (saved to DCIM/Glass):" })
        content.addView(CheckBox(this).apply {
            text = "Sync automatically while Glass is charging on Wi-Fi"
            isChecked = settings.autoSync
            setOnCheckedChangeListener { _, on ->
                settings.autoSync = on
                scope.launch { PhoneHub.media.conditionsChanged() }
            }
        })
        content.addView(Button(this).apply {
            text = "Sync now (Wi-Fi)"
            setOnClickListener { scope.launch { PhoneHub.media.syncNow() } }
        })
        content.addView(status)
        scope.launch { PhoneHub.media.status.collect { status.text = "Sync: $it" } }
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
