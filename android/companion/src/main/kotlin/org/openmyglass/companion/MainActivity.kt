package org.openmyglass.companion

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.app.Dialog
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.media.MediaPlayer
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.OpenableColumns
import android.util.LruCache
import android.view.Gravity
import android.view.View
import android.view.WindowInsetsController
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.CompoundButton
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.MediaController
import android.widget.ScrollView
import android.widget.SeekBar
import android.widget.Spinner
import android.widget.Switch
import android.widget.TextView
import android.widget.VideoView
import org.openmyglass.corebindings.CoreEvent
import org.openmyglass.corebindings.CoreSnapshot
import org.openmyglass.corebindings.GlassRadio
import org.openmyglass.corebindings.GlassSettings
import org.openmyglass.corebindings.LinkPhase
import org.openmyglass.corebindings.NewAttachment
import org.openmyglass.corebindings.NoteAttachmentInfo
import org.openmyglass.corebindings.NoteAttachmentKind
import org.openmyglass.corebindings.NoteInfo
import org.openmyglass.corebindings.NoteLocale
import org.openmyglass.corebindings.NoteOrigin
import org.openmyglass.corebindings.noteLocaleForLanguage
import org.openmyglass.corebindings.timeZoneNames
import java.io.File
import java.io.IOException
import java.text.DateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.Executors
import kotlin.concurrent.thread

/**
 * The first screen of the companion: the link, every Glass setting, and notes. All logic is in
 * the Rust core. This only draws what it reports and calls it.
 */
class MainActivity : Activity(), CompanionScreen {
    private lateinit var process: CompanionProcess
    private val core get() = process.core
    private val link get() = process.link
    private val draft get() = process.draft
    private val main = Handler(Looper.getMainLooper())
    private val prefs by lazy { getSharedPreferences("companion", MODE_PRIVATE) }

    private lateinit var status: TextView
    private lateinit var address: EditText
    private lateinit var settingsBox: LinearLayout
    private lateinit var notesBox: LinearLayout
    private lateinit var draftText: EditText
    private lateinit var draftFiles: LinearLayout
    private lateinit var recordButton: Button
    private lateinit var saveButton: Button
    private lateinit var notesMessage: TextView
    private lateinit var nowPlaying: View
    private lateinit var nowPlayingLabel: TextView
    private lateinit var glassPage: View
    private lateinit var notesPage: View

    private var recorder: VoiceRecorder? = null
    private var recording: File? = null
    private var player: MediaPlayer? = null

    /** The request to read a note, from asking until the audio arrives or the owner stops it. */
    private var speechWanted: ULong? = null

    /** The full-screen photo or video on show. */
    private var viewer: Dialog? = null
    private val thumbnails = LruCache<String, Bitmap>(THUMBNAILS_KEPT)
    private val photoLoader = Executors.newSingleThreadExecutor()

    private val rows = mutableMapOf<String, (GlassSettings) -> Unit>()

    /** While set, a widget changing is us showing the Glass's value, not the owner's wish. */
    private var showing = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        process = try {
            CompanionProcess.of(this)
        } catch (e: org.openmyglass.corebindings.CoreFailure) {
            // Every launch would crash the same way. Say why, and try again on the next one.
            setContentView(
                TextView(this).apply {
                    text = "The companion cannot start: ${e.message}\n\nFree some space on this phone, then open it again."
                    setPadding(dp(16), dp(32), dp(16), dp(16))
                    contentDescription = "startup problem"
                },
            )
            return
        }
        setContentView(buildUi())
        // Needs the window's decor, which setContentView creates. Android 10 keeps its default bars.
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.R) {
            window.insetsController?.setSystemBarsAppearance(
                WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or
                    WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS,
                WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or
                    WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS,
            )
        }
        process.screen = this
        refreshAll()
        // A bonded phone goes back to its Glass by itself, over the link it last used, so waiting
        // notes sync. It does not ask for permissions unasked. Once per process: another screen
        // finds the link already there.
        if (!process.triedBondedGlass) {
            process.triedBondedGlass = true
            onCore({ core.snapshot().bondedTo != null }) { bonded ->
                if (bonded) {
                    if (prefs.getString(KEY_LINK, LINK_TCP) == LINK_BLE) {
                        if (BleLink.missingPermissions(this).isEmpty()) connectBle()
                    } else {
                        connect()
                    }
                }
            }
        }
    }

    override fun onDestroy() {
        if (!::process.isInitialized) {
            super.onDestroy()
            return
        }
        // The link and the draft stay with the process, for the next screen.
        if (process.screen === this) process.screen = null
        finishRecording()
        draft.text = draftText.text.toString()
        stopPlayback()
        viewer?.dismiss()
        photoLoader.shutdownNow()
        super.onDestroy()
    }

    // ------------------------------ the core, away from the main thread ------------------------------

    /** Runs [call] on the core's thread, then [then] here, if this screen is still up. */
    private fun <T> onCore(call: () -> T, failed: () -> Unit = {}, then: (T) -> Unit = {}) {
        process.coreCalls.execute {
            val result = try {
                call()
            } catch (e: Exception) {
                // A panic in the core arrives as uniffi's InternalException. It is not worth the
                // process: say so, and leave the thread to the next call.
                main.post {
                    if (!isDestroyed) {
                        failed()
                        onLinkFailed("The core failed: ${e.message ?: e}")
                    }
                }
                return@execute
            }
            main.post { if (!isDestroyed) then(result) }
        }
    }

    /** Runs [call] on the core's thread and shows why it failed, if it did. */
    private fun tryOnCore(call: () -> Unit) {
        onCore({
            try {
                call()
                null
            } catch (e: org.openmyglass.corebindings.CoreFailure) {
                e.message
            }
        }) { say(it) }
    }

    /** Loads on the core's thread and draws here. Asking while it loads loads once more after. */
    private inner class Refresher<T>(val load: () -> T, val draw: (T) -> Unit) {
        private var loading = false
        private var again = false

        fun run() {
            if (loading) {
                again = true
                return
            }
            loading = true
            onCore(load, failed = { loading = false }) {
                loading = false
                draw(it)
                if (again) {
                    again = false
                    run()
                }
            }
        }
    }

    private val statusRefresher = Refresher({ core.snapshot() }, ::drawStatus)
    private val notesRefresher = Refresher({ core.notes() }, ::drawNotes)

    /** What the core last said, for deciding things on this thread. */
    private var known: CoreSnapshot? = null
    private var knownSettings: GlassSettings? = null

    // ------------------------------ the link ------------------------------

    private fun connect() {
        val text = address.text.toString().trim()
        val host = text.substringBeforeLast(':')
        val port = text.substringAfterLast(':').toIntOrNull()?.takeIf { it in 1..65535 }
        if (host.isEmpty() || port == null) {
            status.text = "Address must be host:port"
            return
        }
        prefs.edit().putString(KEY_ADDRESS, text).putString(KEY_LINK, LINK_TCP).apply()
        linkNote = null
        lastProblem = null
        process.linkRequests++
        link.connect { TcpLink(host, port) }
        refreshStatus()
    }

    private var linkNote: String?
        get() = process.linkNote
        set(value) {
            process.linkNote = value
        }

    /** The link in use is over Bluetooth. Wanting one, or having had one, does not count. */
    private fun overBluetooth(s: CoreSnapshot) = s.phase == LinkPhase.RUNNING && link.running is BleLink

    /** Finds the Glass by its service: no address to type. Asks for the permissions first. */
    private fun connectBle() {
        val missing = BleLink.missingPermissions(this)
        if (missing.isNotEmpty()) {
            requestPermissions(missing.toTypedArray(), REQUEST_BLUETOOTH)
            return
        }
        prefs.edit().putString(KEY_LINK, LINK_BLE).apply()
        linkNote = "Looking for the Glass over Bluetooth"
        lastProblem = null
        process.linkRequests++
        link.connect { cancelled ->
            val address = prefs.getString(KEY_BLE_ADDRESS, null)
                .takeIf { core.snapshot().bondedTo != null }
            BleLink.open(applicationContext, address, cancelled)
        }
        refreshStatus()
    }

    private fun bluetoothAnswered() {
        if (BleLink.missingPermissions(this).isEmpty()) {
            connectBle()
        } else {
            lastProblem = "Bluetooth needs these permissions to find the Glass"
            refreshStatus()
        }
    }

    override fun onLinkFailed(why: String) {
        lastProblem = why
        refreshStatus()
    }

    override fun onCoreEvents(events: List<CoreEvent>, bleAddress: String?) {
        var notes = false
        for (e in events) {
            when (e) {
                is CoreEvent.SettingsChanged -> {
                    knownSettings = e.settings
                    showSettings(e.settings)
                }
                is CoreEvent.NotesChanged -> notes = true
                is CoreEvent.NoteSynced -> {
                    notes = true
                    say(null)
                }
                is CoreEvent.LinkClosed -> e.reason?.let { lastProblem = it }
                is CoreEvent.LinkRunning -> {
                    lastProblem = null
                    // This Glass passed the handshake: the next scan looks for it alone.
                    bleAddress?.let {
                        prefs.edit().putString(KEY_BLE_ADDRESS, it).apply()
                    }
                }
                is CoreEvent.NoteDeferred -> {
                    notes = true
                    say("A note is waiting: ${e.reason}")
                }
                is CoreEvent.NoteRefused -> say("A note was refused: ${e.reason}")
                is CoreEvent.NoteSpeechReady -> if (speechWanted == e.requestId) {
                    speechWanted = null
                    startPlayer(e.path, "Reading the note")
                }
                is CoreEvent.NoteSpeechFailed -> if (speechWanted == e.requestId) {
                    stopPlayback()
                    say("Cannot read it aloud: ${e.reason}")
                }
                else -> Unit
            }
        }
        refreshStatus()
        if (notes) refreshNotes()
    }

    private var lastProblem: String?
        get() = process.lastProblem
        set(value) {
            process.lastProblem = value
        }

    private fun refreshAll() {
        refreshStatus()
        refreshNotes()
    }

    private fun refreshStatus() = statusRefresher.run()

    private fun drawStatus(s: CoreSnapshot) {
        // A new screen has missed the event that carried these.
        if (known == null) s.glass.settings?.let { showSettings(it) }
        known = s
        knownSettings = s.glass.settings
        val g = s.glass
        status.text = buildString {
            append(
                when (s.phase) {
                    LinkPhase.DOWN -> linkNote ?: "Not connected"
                    LinkPhase.HANDSHAKE -> "Connecting"
                    LinkPhase.RUNNING ->
                        "Connected to ${g.name ?: "Glass"}" +
                            if (overBluetooth(s)) " over Bluetooth" else ""
                },
            )
            s.bondedTo?.let { append("\nPaired with $it") }
            g.battery_line()?.let { append("\n$it") }
            g.softwareVersion?.let { append("\n$it") }
            g.pendingNotes?.let { count ->
                append("\nNotes waiting on the Glass: $count")
                g.pendingBytes?.takeIf { it > 0u }?.let { append(", ${megabytes(it)}") }
            }
            s.receiving?.let { append("\nReceiving a note: ${megabytes(it.receivedBytes)} of ${megabytes(it.totalBytes)}") }
            if (s.notesSetAside > 0u) append("\nNotes that could not be read and were set aside: ${s.notesSetAside}")
            lastProblem?.let { append("\n$it") }
        }
        // The core keeps the last values after a link ends, and a change would go nowhere then.
        val up = s.phase == LinkPhase.RUNNING
        if (!up) showSettings(GlassSettings())
        wifi.setShown(g.wifiOn.takeIf { up })
        bluetooth.setShown(g.bluetoothOn.takeIf { up })
        airplane.setShown(g.airplaneMode.takeIf { up })
    }

    private fun org.openmyglass.corebindings.GlassState.battery_line(): String? =
        batteryPercent?.let { "Battery $it%" + if (charging == true) ", charging" else "" }

    // ------------------------------ layout ------------------------------

    private fun buildUi(): View {
        val root = column().apply { setPadding(dp(12), dp(8), dp(12), dp(8)) }
        status = TextView(this).apply { contentDescription = "status" }
        address = EditText(this).apply {
            setSingleLine()
            hint = "host:port"
            setText(prefs.getString(KEY_ADDRESS, DEFAULT_ADDRESS))
            contentDescription = "address"
        }
        root.addView(status)
        root.addView(address)
        root.addView(
            row(
                button("Connect") { connect() },
                button("Bluetooth") { connectBle() },
                button("Disconnect") {
                    linkNote = null
                    lastProblem = null
                    process.linkRequests++
                    link.disconnect()
                    refreshStatus()
                },
                button("Unpair") {
                    // Waits behind other calls. A link the owner asks for meanwhile is newer than
                    // this, and must not be ended by it.
                    val asked = process.linkRequests
                    onCore({
                        // If this phone cannot forget the bond, the core tells the Glass nothing
                        // and the link stays, so both ends still agree.
                        try {
                            core.unpair()
                            prefs.edit().remove(KEY_BLE_ADDRESS).apply()
                            // The Glass forgets this phone only if the core's last frame reaches it.
                            // On the main thread, where requests are made, so none slips in between.
                            main.post {
                                if (process.linkRequests == asked) link.disconnectAfterFlush()
                            }
                            null
                        } catch (e: org.openmyglass.corebindings.CoreFailure) {
                            "Could not forget the Glass on this phone: ${e.message}"
                        }
                    }) { problem ->
                        lastProblem = problem
                        if (problem == null) linkNote = null
                        refreshStatus()
                    }
                },
            ),
        )
        glassPage = ScrollView(this).apply { addView(buildGlassPage()) }
        notesPage = ScrollView(this).apply { addView(buildNotesPage()) }
        root.addView(
            row(
                button("Glass") { showPage(glassPage) },
                button("Notes") { showPage(notesPage) },
            ),
        )
        root.addView(glassPage, LinearLayout.LayoutParams(MATCH_PARENT, 0, 1f))
        root.addView(notesPage, LinearLayout.LayoutParams(MATCH_PARENT, 0, 1f))
        showPage(glassPage)
        // fitsSystemWindows replaces a view's padding, so it goes on a wrapper.
        return FrameLayout(this).apply {
            fitsSystemWindows = true
            addView(root)
        }
    }

    private fun showPage(page: View) {
        glassPage.visibility = if (page === glassPage) View.VISIBLE else View.GONE
        notesPage.visibility = if (page === notesPage) View.VISIBLE else View.GONE
    }

    // ------------------------------ Glass settings ------------------------------

    private lateinit var wifi: Switch
    private lateinit var bluetooth: Switch
    private lateinit var airplane: Switch

    private fun buildGlassPage(): View {
        settingsBox = column()
        heading("Radios")
        wifi = plainSwitch("Wi-Fi") { on -> whenRunning { core.setRadio(GlassRadio.WIFI, on) } }
        // On the Glass, Bluetooth runs when it is wanted and airplane mode is off.
        bluetooth = plainSwitch("Bluetooth") { on ->
            confirmIfItDropsTheLink(!on, "Turning Bluetooth off", "turn Bluetooth back on") {
                whenRunning { core.setRadio(GlassRadio.BLUETOOTH, on) }
            }
        }
        airplane = plainSwitch("Airplane mode") { on ->
            confirmIfItDropsTheLink(on, "Turning airplane mode on", "turn airplane mode off") {
                whenRunning { core.setAirplaneMode(on) }
            }
        }

        heading("Screen and waking")
        number("brightness", "Brightness", 0, 100, { it.brightness }) { GlassSettings(brightness = it) }
        number("screen_timeout", "Screen timeout (s)", 3, 600, { it.screenTimeoutS }) {
            GlassSettings(screenTimeoutS = it)
        }
        toggle("head_wake", "Wake on looking up", { it.headWake }) { GlassSettings(headWake = it) }
        number("head_wake_angle", "Look-up angle (degrees)", 10, 40, { it.headWakeAngleDeg }) {
            GlassSettings(headWakeAngleDeg = it)
        }
        toggle("nudge", "Head nudge", { it.nudge }) { GlassSettings(nudge = it) }
        toggle("wink", "Wink to capture", { it.wink }) { GlassSettings(wink = it) }
        toggle("on_head", "On-head detection", { it.onHeadDetection }) {
            GlassSettings(onHeadDetection = it)
        }

        heading("Sound")
        number("volume", "Volume (%)", 0, 100, { it.audioVolume }) { GlassSettings(audioVolume = it) }

        heading("Energy")
        toggle("power_saver", "Energy saver", { it.powerSaver }) { GlassSettings(powerSaver = it) }
        toggle("deep_sleep", "Deep sleep", { it.deepSleep }) { GlassSettings(deepSleep = it) }
        number("deep_sleep_idle", "Idle before deep sleep (s)", 5, 1800, { it.deepSleepIdleS }) {
            GlassSettings(deepSleepIdleS = it)
        }
        // The launcher's choices. A value set elsewhere (glassd allows up to 720) shows as "Other",
        // and picking "Other" leaves it unchanged.
        val powerOffMinutes = listOf(10u, 30u, 60u, 120u, 0u)
        choice(
            "off_head_power_off",
            "Power off when removed",
            listOf("10 min", "30 min", "60 min", "120 min", "Never", "Other"),
            { s ->
                s.offHeadPowerOffMin?.let { m ->
                    powerOffMinutes.indexOf(m).takeIf { it >= 0 } ?: powerOffMinutes.size
                }
            },
        ) { position ->
            powerOffMinutes.getOrNull(position)?.let { GlassSettings(offHeadPowerOffMin = it) }
        }

        heading("Time and language")
        toggle("time_24h", "24-hour clock", { it.time24h }) { GlassSettings(time24h = it) }
        val zones = timeZoneNames()
        choice("timezone", "Time zone", zones, { it.timezoneId?.toInt() }) {
            GlassSettings(timezoneId = it.toUInt())
        }
        // The Glass reports no language until someone chooses one, and it can be chosen from here.
        val locales = NoteLocale.values()
        choice("locale", "Language", listOf("Not chosen") + locales.map { LOCALE_NAMES.getValue(it) }, {
            when {
                it.screenTimeoutS == null -> null
                else -> it.locale?.let { l -> locales.indexOf(l) + 1 } ?: 0
            }
        }) { position -> locales.getOrNull(position - 1)?.let { GlassSettings(locale = it) } }
        return settingsBox
    }

    private fun showSettings(s: GlassSettings) {
        showing = true
        rows.values.forEach { it(s) }
        showing = false
    }

    private fun send(change: GlassSettings) {
        if (showing) return
        whenRunning { core.changeSettings(change) }
    }

    /** The core drops a change with no running link, so say so instead. */
    private fun whenRunning(change: () -> Unit) {
        if (known?.phase == LinkPhase.RUNNING) {
            onCore({
                change()
                link.poke()
            })
        } else {
            lastProblem = "Not sent: the Glass is not connected"
            refreshStatus()
        }
    }

    private fun heading(text: String) {
        settingsBox.addView(
            TextView(this).apply {
                this.text = text
                setPadding(0, dp(12), 0, dp(2))
                setTypeface(typeface, android.graphics.Typeface.BOLD)
            },
        )
    }

    /**
     * This phone reaches the Glass over the radio the owner is about to turn off, and only the
     * Glass itself can then turn it back on: ask first, and on "no" show what the Glass reports.
     */
    private fun confirmIfItDropsTheLink(dropsRadio: Boolean, what: String, undo: String, apply: () -> Unit) {
        val s = known
        if (!dropsRadio || s == null || !overBluetooth(s)) {
            apply()
            return
        }
        AlertDialog.Builder(this)
            .setTitle("Disconnect from the Glass?")
            .setMessage(
                "This phone is connected over Bluetooth, so $what will end the connection. " +
                    "To connect again, $undo from the settings on the Glass.",
            )
            .setPositiveButton("Continue") { _, _ -> apply() }
            .setNegativeButton("Cancel") { _, _ -> refreshStatus() }
            .setOnCancelListener { refreshStatus() }
            .show()
    }

    private fun plainSwitch(label: String, onChange: (Boolean) -> Unit): Switch {
        val view = Switch(this).apply {
            text = label
            contentDescription = label
            visibility = View.GONE
            setOnCheckedChangeListener { _: CompoundButton, on: Boolean -> if (!showing) onChange(on) }
        }
        settingsBox.addView(view)
        return view
    }

    private fun Switch.setShown(value: Boolean?) {
        visibility = if (value == null) View.GONE else View.VISIBLE
        if (value != null && isChecked != value) {
            showing = true
            isChecked = value
            showing = false
        }
    }

    private fun toggle(
        key: String,
        label: String,
        read: (GlassSettings) -> Boolean?,
        change: (Boolean) -> GlassSettings,
    ) {
        val view = plainSwitch(label) { send(change(it)) }
        rows[key] = { s -> view.setShown(read(s)) }
    }

    private fun number(
        key: String,
        label: String,
        min: Int,
        max: Int,
        read: (GlassSettings) -> UInt?,
        change: (UInt) -> GlassSettings,
    ) {
        val title = TextView(this).apply { visibility = View.GONE }
        val seek = SeekBar(this).apply {
            this.min = min
            this.max = max
            visibility = View.GONE
            contentDescription = label
            setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
                override fun onProgressChanged(bar: SeekBar, value: Int, fromUser: Boolean) {
                    title.text = "$label: $value"
                }

                override fun onStartTrackingTouch(bar: SeekBar) = Unit
                override fun onStopTrackingTouch(bar: SeekBar) = send(change(bar.progress.toUInt()))
            })
        }
        settingsBox.addView(title)
        settingsBox.addView(seek)
        rows[key] = { s ->
            val value = read(s)
            val shown = if (value == null) View.GONE else View.VISIBLE
            title.visibility = shown
            seek.visibility = shown
            if (value != null) {
                seek.progress = value.toInt()
                title.text = "$label: $value"
            }
        }
    }

    private fun choice(
        key: String,
        label: String,
        options: List<String>,
        read: (GlassSettings) -> Int?,
        change: (Int) -> GlassSettings?,
    ) {
        val title = TextView(this).apply { text = label; visibility = View.GONE }
        val spinner = Spinner(this).apply {
            adapter = ArrayAdapter(context, android.R.layout.simple_spinner_dropdown_item, options)
            visibility = View.GONE
            contentDescription = label
            onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
                override fun onItemSelected(p: AdapterView<*>?, v: View?, position: Int, id: Long) {
                    val settings = knownSettings ?: return
                    val current = read(settings)
                    if (current == position) return
                    // An entry that only describes a state ("Other") goes back to the real value.
                    change(position)?.let { send(it) } ?: current?.let { p?.setSelection(it) }
                }

                override fun onNothingSelected(p: AdapterView<*>?) = Unit
            }
        }
        settingsBox.addView(title)
        settingsBox.addView(spinner)
        rows[key] = { s ->
            val value = read(s)
            val shown = if (value == null) View.GONE else View.VISIBLE
            title.visibility = shown
            spinner.visibility = shown
            if (value != null && value in options.indices) spinner.setSelection(value, false)
        }
    }

    private fun lastSettings(): GlassSettings = knownSettings ?: GlassSettings()

    // ------------------------------ notes ------------------------------

    private fun buildNotesPage(): View {
        val page = column()
        draftText = EditText(this).apply {
            hint = "A new note"
            contentDescription = "note text"
        }
        draftText.setText(draft.text)
        draftFiles = column().apply { contentDescription = "note attachments" }
        recordButton = button("Record voice") { toggleRecording() }
        saveButton = button("Save note") { saveNote() }
        notesMessage = TextView(this).apply {
            contentDescription = "notes message"
            visibility = View.GONE
        }
        nowPlayingLabel = TextView(this)
        nowPlaying = row(nowPlayingLabel, button("Stop") { stopPlayback() }).apply { visibility = View.GONE }
        notesBox = column()
        page.addView(draftText)
        page.addView(
            row(
                recordButton,
                button("Photos") {
                    pick("image/*", multiple = true, REQUEST_PHOTOS)
                },
                button("Video") {
                    pick("video/*", multiple = false, REQUEST_VIDEO)
                },
            ),
        )
        page.addView(draftFiles)
        page.addView(saveButton)
        page.addView(notesMessage)
        page.addView(nowPlaying)
        page.addView(
            TextView(this).apply {
                text = "Notes"
                setPadding(0, dp(12), 0, 0)
                setTypeface(typeface, android.graphics.Typeface.BOLD)
            },
        )
        page.addView(notesBox)
        showDraft()
        return page
    }

    private fun pick(type: String, multiple: Boolean, request: Int) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT)
            .addCategory(Intent.CATEGORY_OPENABLE)
            .setType(type)
            .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, multiple)
        startActivityForResult(intent, request)
    }

    @Deprecated("Deprecated in Android")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        // The core did not start, and this screen only says so.
        if (!::process.isInitialized) return
        if (resultCode != RESULT_OK || data == null) return
        val uris = data.clipData?.let { clip -> (0 until clip.itemCount).map { clip.getItemAt(it).uri } }
            ?: listOfNotNull(data.data)
        val kind = if (requestCode == REQUEST_VIDEO) NoteAttachmentKind.VIDEO else NoteAttachmentKind.PHOTO
        say("Copying")
        thread {
            val added = uris.mapNotNull { copyIn(it, kind) }
            main.post {
                draft.files.addAll(added)
                if (!isDestroyed) {
                    if (added.size == uris.size) say(null)
                    showDraft()
                }
            }
        }
    }

    /** The core needs a path, and a picker gives a URI. The copy goes when the note is saved. */
    private fun copyIn(uri: Uri, kind: NoteAttachmentKind): NewAttachment? {
        var target: File? = null
        val problem = try {
            val name = contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
                ?.use { if (it.moveToFirst()) it.getString(0) else null } ?: "file"
            val copy = File.createTempFile("pick", "-" + name.takeLast(40).replace('/', '_'), cacheDir)
            target = copy
            val source = contentResolver.openInputStream(uri) ?: throw IOException("it is gone")
            source.use { input ->
                copy.outputStream().use { input.copyTo(it) }
            }
            return NewAttachment(kind, contentResolver.getType(uri) ?: "application/octet-stream", copy.path, null)
        } catch (e: IOException) {
            e
        } catch (e: RuntimeException) {
            // What another app's provider throws is that app's problem, not a reason to crash.
            e
        }
        target?.delete()
        main.post { if (!isDestroyed) say("Cannot read that file: ${problem.message}") }
        return null
    }

    private fun toggleRecording() {
        if (recorder != null) {
            finishRecording()
            return
        }
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), REQUEST_MIC)
            return
        }
        val file = try {
            File.createTempFile("voice", ".wav", cacheDir)
        } catch (e: IOException) {
            say("Cannot record: ${e.message}")
            return
        }
        val started = VoiceRecorder(file)
        if (!started.start()) {
            file.delete()
            say("The microphone is not available")
            return
        }
        recording = file
        recorder = started
        recordButton.text = "Stop"
    }

    /** Puts what was recorded in the draft, if a recording is running. */
    private fun finishRecording() {
        val running = recorder ?: return
        val file = recording ?: return
        val length = running.stop()
        recorder = null
        recording = null
        recordButton.text = "Record voice"
        if (length == null) {
            file.delete()
            say("The recording failed")
            return
        }
        draft.files.add(NewAttachment(NoteAttachmentKind.VOICE, "audio/wav", file.path, length.toUInt()))
        showDraft()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<String>, results: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, results)
        if (!::process.isInitialized) return
        if (requestCode == REQUEST_MIC) {
            if (results.firstOrNull() == PackageManager.PERMISSION_GRANTED) {
                toggleRecording()
            } else {
                say("Recording a voice note needs the microphone permission")
            }
        }
        if (requestCode == REQUEST_BLUETOOTH) bluetoothAnswered()
    }

    /** The draft as the process holds it. While it saves, nothing that would change it works. */
    private fun showDraft() {
        draftText.isEnabled = !draft.saving
        saveButton.isEnabled = !draft.saving
        draftFiles.removeAllViews()
        draft.files.toList().forEach { file ->
            val name = File(file.path).name.let { it.substringAfter('-', it) }
            val label = TextView(this).apply { text = "${file.kind.name.lowercase()}: $name" }
            val remove = button("Remove") {
                draft.files.remove(file)
                File(file.path).delete()
                showDraft()
            }
            remove.isEnabled = !draft.saving
            draftFiles.addView(row(label, remove))
        }
    }

    private fun saveNote() {
        finishRecording()
        draft.text = draftText.text.toString()
        // With no link the Glass cannot say, and a note in the wrong language transcribes badly.
        val locale = lastSettings().locale ?: noteLocaleForLanguage(Locale.getDefault().language)
        process.saveDraft(locale)
        showDraft()
    }

    override fun onDraftChanged(problem: String?) {
        say(problem)
        draftText.setText(draft.text)
        showDraft()
        refreshNotes()
    }

    private fun refreshNotes() = notesRefresher.run()

    private fun drawNotes(notes: List<NoteInfo>) {
        notesBox.removeAllViews()
        if (notes.isEmpty()) {
            notesBox.addView(TextView(this).apply { text = "None yet." })
        }
        notes.forEach { notesBox.addView(noteView(it)) }
    }

    private fun noteView(n: NoteInfo): View {
        val box = column().apply { setPadding(0, dp(8), 0, dp(8)); contentDescription = "note ${n.id}" }
        val date = DateFormat.getDateTimeInstance().format(Date(n.createdAtMs.toLong()))
        val from = if (n.origin == NoteOrigin.GLASS) "from the Glass" else "on this phone"
        val state = if (n.complete || n.waiting != null) "" else ", still arriving"
        box.addView(small("$date, $from$state"))
        n.waiting?.let { box.addView(small("Waiting: $it")) }
        n.text?.let { box.addView(selectable(it)) }
        n.transcript?.let { heard ->
            box.addView(if (heard.isBlank()) small("Nothing was heard in the recording.") else selectable("Heard: $heard"))
        }
        if (n.transcribing) box.addView(small("Transcribing the recording"))
        n.transcriptError?.let { why ->
            box.addView(small("No transcript: $why"))
            if (!n.transcribing) box.addView(button("Transcribe again") { transcribeAgain(n) })
        }
        n.attachments.forEach { box.addView(attachmentView(it)) }
        val actions = mutableListOf<View>()
        if (n.speakable) actions.add(button("Read aloud") { readAloud(n) })
        if (n.deletable) actions.add(button("Delete") { confirmDelete(n) })
        if (actions.isNotEmpty()) box.addView(row(*actions.toTypedArray()))
        return box
    }

    private fun small(text: String) = TextView(this).apply {
        this.text = text
        textSize = 12f
    }

    private fun selectable(text: String) = TextView(this).apply {
        this.text = text
        setTextIsSelectable(true)
    }

    private fun attachmentView(a: NoteAttachmentInfo): View {
        val path = a.path
        val size = megabytes(a.size)
        if (path == null) return small("${a.kind.name.lowercase()}, $size, not here yet")
        return when (a.kind) {
            NoteAttachmentKind.PHOTO -> thumbnail(path)
            NoteAttachmentKind.VOICE -> button("Play recording, $size") { playRecording(path) }
            NoteAttachmentKind.VIDEO -> button("Play video, $size") { showVideo(path) }
        }
    }

    private fun thumbnail(path: String): View {
        val view = ImageView(this).apply {
            adjustViewBounds = true
            maxHeight = dp(160)
            contentDescription = "photo"
            setOnClickListener { showPhoto(path) }
        }
        val known = thumbnails.get(path)
        if (known != null) {
            view.setImageBitmap(known)
            return view
        }
        val maxSide = dp(320)
        photoLoader.execute {
            val bitmap = thumbnails.get(path) ?: Photos.decode(path, maxSide)
            main.post {
                if (isDestroyed) return@post
                if (bitmap == null) {
                    view.setImageResource(android.R.drawable.ic_menu_report_image)
                    view.contentDescription = "a photo that cannot be shown"
                } else {
                    thumbnails.put(path, bitmap)
                    view.setImageBitmap(bitmap)
                }
            }
        }
        return view
    }

    private fun fullScreen(content: View): Dialog {
        viewer?.dismiss()
        return Dialog(this, android.R.style.Theme_Black_NoTitleBar_Fullscreen).apply {
            setContentView(content)
            viewer = this
        }
    }

    private fun showPhoto(path: String) {
        val view = ImageView(this).apply { contentDescription = "photo, full size" }
        val dialog = fullScreen(view)
        view.setOnClickListener { dialog.dismiss() }
        dialog.show()
        val metrics = resources.displayMetrics
        val maxSide = maxOf(metrics.widthPixels, metrics.heightPixels)
        photoLoader.execute {
            val bitmap = Photos.decode(path, maxSide)
            main.post {
                if (isDestroyed) return@post
                if (bitmap == null) {
                    dialog.dismiss()
                    say("This photo cannot be shown")
                } else {
                    view.setImageBitmap(bitmap)
                }
            }
        }
    }

    private fun showVideo(path: String) {
        stopPlayback()
        val video = VideoView(this).apply { contentDescription = "video" }
        val holder = FrameLayout(this).apply {
            addView(video, FrameLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT, Gravity.CENTER))
        }
        val dialog = fullScreen(holder)
        val controls = MediaController(this)
        controls.setAnchorView(video)
        video.setMediaController(controls)
        video.setOnPreparedListener { video.start() }
        video.setOnErrorListener { _, _, _ ->
            dialog.dismiss()
            say("This phone cannot play this video")
            true
        }
        dialog.setOnDismissListener { video.stopPlayback() }
        video.setVideoPath(path)
        dialog.show()
    }

    private fun transcribeAgain(n: NoteInfo) {
        tryOnCore {
            core.retranscribeNote(n.id)
            link.poke()
        }
        refreshNotes()
    }

    private fun confirmDelete(n: NoteInfo) {
        AlertDialog.Builder(this)
            .setTitle("Delete this note?")
            .setMessage("It is removed from this phone, with its attachments.")
            .setPositiveButton("Delete") { _, _ -> delete(n) }
            .setNegativeButton("Cancel", null)
            .show()
    }

    private fun delete(n: NoteInfo) {
        stopPlayback()
        n.attachments.mapNotNull { it.path }.forEach { thumbnails.remove(it) }
        // Its own events would also say so, but only once the poll comes round.
        tryOnCore { core.deleteNote(n.id) }
        refreshNotes()
    }

    private fun readAloud(n: NoteInfo) {
        stopPlayback()
        val request = ++process.lastSpeechRequest
        speechWanted = request
        say(null)
        showNowPlaying("Preparing speech")
        onCore({
            try {
                core.speakNote(n.id, request)
                null
            } catch (e: org.openmyglass.corebindings.CoreFailure) {
                e.message
            }
        }) { problem ->
            if (problem != null && speechWanted == request) {
                stopPlayback()
                say(problem)
            }
        }
    }

    private fun playRecording(path: String) {
        stopPlayback()
        startPlayer(path, "Playing the recording")
    }

    private fun startPlayer(path: String, label: String) {
        player?.release()
        player = null
        val next = MediaPlayer()
        try {
            next.setDataSource(path)
            next.setOnCompletionListener { stopPlayback() }
            next.prepare()
            next.start()
            player = next
            say(null)
            showNowPlaying(label)
        } catch (e: IOException) {
            next.release()
            stopPlayback()
            say("Cannot play it: ${e.message}")
        } catch (e: IllegalStateException) {
            next.release()
            stopPlayback()
            say("Cannot play it: ${e.message}")
        }
    }

    private fun showNowPlaying(label: String) {
        nowPlayingLabel.text = label
        nowPlaying.visibility = View.VISIBLE
    }

    private fun stopPlayback() {
        player?.release()
        player = null
        speechWanted = null
        onCore({ core.stopSpeaking() })
        nowPlaying.visibility = View.GONE
    }

    /** A line about notes above the list, or none. */
    private fun say(message: String?) {
        notesMessage.text = message.orEmpty()
        notesMessage.visibility = if (message == null) View.GONE else View.VISIBLE
    }

    private fun megabytes(bytes: ULong) = "%.2f MB".format(bytes.toLong() / 1_000_000.0)

    // ------------------------------ helpers ------------------------------

    private fun column() = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }

    private fun row(vararg views: View) = LinearLayout(this).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        views.forEach { addView(it, LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f)) }
    }

    private fun button(label: String, onClick: () -> Unit) = Button(this).apply {
        text = label
        isAllCaps = false
        contentDescription = label
        setOnClickListener { onClick() }
    }

    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    private companion object {
        const val KEY_ADDRESS = "address"
        const val KEY_LINK = "link"
        const val KEY_BLE_ADDRESS = "ble_address"
        const val LINK_TCP = "tcp"
        const val LINK_BLE = "ble"

        /** The other end of `adb reverse tcp:7700 tcp:7700`, until the Glass is reachable. */
        const val DEFAULT_ADDRESS = "127.0.0.1:7700"
        const val REQUEST_PHOTOS = 1
        const val REQUEST_VIDEO = 2
        const val REQUEST_MIC = 3
        const val REQUEST_BLUETOOTH = 4
        const val THUMBNAILS_KEPT = 64
        val LOCALE_NAMES = mapOf(
            NoteLocale.IT to "Italiano",
            NoteLocale.EN to "English",
            NoteLocale.FR to "Français",
            NoteLocale.ES to "Español",
        )
    }
}
