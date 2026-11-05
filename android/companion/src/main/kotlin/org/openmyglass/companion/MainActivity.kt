package org.openmyglass.companion

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.BitmapFactory
import android.media.MediaPlayer
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.OpenableColumns
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
import android.widget.ScrollView
import android.widget.SeekBar
import android.widget.Spinner
import android.widget.Switch
import android.widget.TextView
import org.openmyglass.corebindings.CompanionCore
import org.openmyglass.corebindings.CoreEvent
import org.openmyglass.corebindings.GlassRadio
import org.openmyglass.corebindings.GlassSettings
import org.openmyglass.corebindings.LinkPhase
import org.openmyglass.corebindings.NewAttachment
import org.openmyglass.corebindings.NoteAttachmentInfo
import org.openmyglass.corebindings.NoteAttachmentKind
import org.openmyglass.corebindings.NoteInfo
import org.openmyglass.corebindings.NoteLocale
import org.openmyglass.corebindings.NoteOrigin
import org.openmyglass.corebindings.timeZoneNames
import java.io.File
import java.text.DateFormat
import java.util.Date
import kotlin.concurrent.thread

/**
 * The first screen of the companion: the link, every Glass setting, and notes. All logic is in
 * the Rust core. This only draws what it reports and calls it.
 */
class MainActivity : Activity() {
    private lateinit var core: CompanionCore
    private lateinit var link: LinkController
    private val main = Handler(Looper.getMainLooper())
    private val prefs by lazy { getSharedPreferences("companion", MODE_PRIVATE) }

    private lateinit var status: TextView
    private lateinit var address: EditText
    private lateinit var settingsBox: LinearLayout
    private lateinit var notesBox: LinearLayout
    private lateinit var draftText: EditText
    private lateinit var draftFiles: TextView
    private lateinit var recordButton: Button
    private lateinit var glassPage: View
    private lateinit var notesPage: View

    private val pending = mutableListOf<NewAttachment>()
    private var recorder: VoiceRecorder? = null
    private var recording: File? = null
    private var player: MediaPlayer? = null

    private val rows = mutableMapOf<String, (GlassSettings) -> Unit>()

    /** While set, a widget changing is us showing the Glass's value, not the owner's wish. */
    private var showing = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val engineContext = applicationContext
        core = CompanionCore(
            File(filesDir, "companion").path,
            android.os.Build.MODEL,
            packageManager.getPackageInfo(packageName, 0).versionName ?: "0",
            AndroidStt(engineContext),
            AndroidTts(engineContext),
        )
        link = LinkController(core, { filesDir.usableSpace }) { events ->
            main.post { onEvents(events) }
        }
        setContentView(buildUi())
        // Needs the window's decor, which setContentView creates.
        window.insetsController?.setSystemBarsAppearance(
            WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or
                WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS,
            WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or
                WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS,
        )
        refreshAll()
        // A bonded phone goes back to its Glass by itself, over the link it last used, so waiting
        // notes sync. It does not ask for permissions unasked.
        if (core.snapshot().bondedTo != null) {
            if (prefs.getString(KEY_LINK, LINK_TCP) == LINK_BLE) {
                if (BleLink.missingPermissions(this).isEmpty()) connectBle()
            } else {
                connect()
            }
        }
    }

    override fun onDestroy() {
        bleAttempt = null
        link.disconnect()
        player?.release()
        super.onDestroy()
    }

    // ------------------------------ the link ------------------------------

    private fun connect() {
        val text = address.text.toString().trim()
        val host = text.substringBeforeLast(':')
        val port = text.substringAfterLast(':').toIntOrNull()
        if (host.isEmpty() || port == null) {
            status.text = "Address must be host:port"
            return
        }
        prefs.edit().putString(KEY_ADDRESS, text).putString(KEY_LINK, LINK_TCP).apply()
        bleAttempt = null
        linkNote = null
        link.connect { TcpLink(host, port) }
        refreshStatus()
    }

    /** Set while a BLE link is wanted. A scan in progress stops when this is no longer its own. */
    @Volatile private var bleAttempt: Any? = null

    /** The address of the BLE link in use, saved once the Glass has proved itself. */
    @Volatile private var bleAddress: String? = null
    private var linkNote: String? = null

    /** Finds the Glass by its service: no address to type. Asks for the permissions first. */
    private fun connectBle() {
        val missing = BleLink.missingPermissions(this)
        if (missing.isNotEmpty()) {
            requestPermissions(missing.toTypedArray(), REQUEST_BLUETOOTH)
            return
        }
        prefs.edit().putString(KEY_LINK, LINK_BLE).apply()
        val attempt = Any()
        bleAttempt = attempt
        linkNote = "Looking for the Glass over Bluetooth"
        val known = prefs.getString(KEY_BLE_ADDRESS, null)
            .takeIf { core.snapshot().bondedTo != null }
        link.connect {
            BleLink.open(applicationContext, known) { bleAttempt !== attempt }.also {
                bleAddress = it.address
            }
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

    private fun onEvents(events: List<CoreEvent>) {
        var notes = false
        for (e in events) {
            when (e) {
                is CoreEvent.SettingsChanged -> showSettings(e.settings)
                is CoreEvent.NotesChanged, is CoreEvent.NoteSynced -> notes = true
                is CoreEvent.LinkClosed -> e.reason?.let { lastProblem = it }
                is CoreEvent.LinkRunning -> {
                    lastProblem = null
                    // This Glass passed the handshake: the next scan looks for it alone.
                    if (bleAttempt != null) {
                        bleAddress?.let { prefs.edit().putString(KEY_BLE_ADDRESS, it).apply() }
                    }
                }
                is CoreEvent.NoteDeferred -> lastProblem = "Note waiting: ${e.reason}"
                is CoreEvent.NoteRefused -> lastProblem = "Note refused: ${e.reason}"
                else -> Unit
            }
        }
        refreshStatus()
        // Without a link a change would go nowhere, and the old values may no longer be true.
        if (core.snapshot().phase != LinkPhase.RUNNING) showSettings(GlassSettings())
        if (notes) refreshNotes()
    }

    private var lastProblem: String? = null

    private fun refreshAll() {
        refreshStatus()
        refreshNotes()
    }

    private fun refreshStatus() {
        val s = core.snapshot()
        val g = s.glass
        status.text = buildString {
            append(
                when (s.phase) {
                    LinkPhase.DOWN -> linkNote?.takeIf { bleAttempt != null } ?: "Not connected"
                    LinkPhase.HANDSHAKE -> "Connecting"
                    LinkPhase.RUNNING ->
                        "Connected to ${g.name ?: "Glass"}" +
                            if (bleAttempt != null) " over Bluetooth" else ""
                },
            )
            s.bondedTo?.let { append("\nPaired with $it") }
            g.battery_line()?.let { append("\n$it") }
            g.softwareVersion?.let { append("\n$it") }
            g.pendingNotes?.let { append("\nNotes waiting on the Glass: $it") }
            lastProblem?.let { append("\n$it") }
        }
        wifi.setShown(g.wifiOn)
        bluetooth.setShown(g.bluetoothOn)
        airplane.setShown(g.airplaneMode)
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
                    bleAttempt = null
                    link.disconnect()
                    refreshStatus()
                },
                button("Unpair") {
                    core.unpair()
                    prefs.edit().remove(KEY_BLE_ADDRESS).apply()
                    bleAttempt = null
                    link.poke()
                    main.postDelayed({ link.disconnect(); refreshStatus() }, 300)
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
        wifi = plainSwitch("Wi-Fi") { core.setRadio(GlassRadio.WIFI, it); link.poke() }
        bluetooth = plainSwitch("Bluetooth") { core.setRadio(GlassRadio.BLUETOOTH, it); link.poke() }
        airplane = plainSwitch("Airplane mode") { core.setAirplaneMode(it); link.poke() }

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
        core.changeSettings(change)
        link.poke()
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
                    val known = core.snapshot().glass.settings ?: return
                    val current = read(known)
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

    private fun lastSettings(): GlassSettings = core.snapshot().glass.settings ?: GlassSettings()

    // ------------------------------ notes ------------------------------

    private fun buildNotesPage(): View {
        val page = column()
        draftText = EditText(this).apply {
            hint = "A new note"
            contentDescription = "note text"
        }
        draftFiles = TextView(this).apply { contentDescription = "note attachments" }
        recordButton = button("Record voice") { toggleRecording() }
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
        page.addView(button("Save note") { saveNote() })
        page.addView(
            TextView(this).apply {
                text = "Notes"
                setPadding(0, dp(12), 0, 0)
                setTypeface(typeface, android.graphics.Typeface.BOLD)
            },
        )
        page.addView(notesBox)
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
        if (resultCode != RESULT_OK || data == null) return
        val uris = data.clipData?.let { clip -> (0 until clip.itemCount).map { clip.getItemAt(it).uri } }
            ?: listOfNotNull(data.data)
        val kind = if (requestCode == REQUEST_VIDEO) NoteAttachmentKind.VIDEO else NoteAttachmentKind.PHOTO
        thread {
            val added = uris.mapNotNull { copyIn(it, kind) }
            main.post {
                pending.addAll(added)
                showDraftFiles()
            }
        }
    }

    /** The core needs a path, and a picker gives a URI. The copy goes when the note is saved. */
    private fun copyIn(uri: Uri, kind: NoteAttachmentKind): NewAttachment? = try {
        val name = contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
            ?.use { if (it.moveToFirst()) it.getString(0) else null } ?: "file"
        val target = File.createTempFile("pick", "-" + name.takeLast(40).replace('/', '_'), cacheDir)
        contentResolver.openInputStream(uri)!!.use { input ->
            target.outputStream().use { input.copyTo(it) }
        }
        NewAttachment(
            kind,
            contentResolver.getType(uri) ?: "application/octet-stream",
            target.path,
            null,
        )
    } catch (e: java.io.IOException) {
        main.post { lastProblem = "Cannot read that file: ${e.message}"; refreshStatus() }
        null
    }

    private fun toggleRecording() {
        val running = recorder
        if (running != null) {
            val length = running.stop()
            recorder = null
            recordButton.text = "Record voice"
            pending.add(
                NewAttachment(NoteAttachmentKind.VOICE, "audio/wav", recording!!.path, length.toUInt()),
            )
            showDraftFiles()
            return
        }
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), REQUEST_MIC)
            return
        }
        val file = File.createTempFile("voice", ".wav", cacheDir)
        recording = file
        recorder = VoiceRecorder(file).also { it.start() }
        recordButton.text = "Stop"
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<String>, results: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, results)
        if (requestCode == REQUEST_MIC && results.firstOrNull() == PackageManager.PERMISSION_GRANTED) {
            toggleRecording()
        }
        if (requestCode == REQUEST_BLUETOOTH) bluetoothAnswered()
    }

    private fun showDraftFiles() {
        draftFiles.text = pending.joinToString("\n") { "${it.kind.name.lowercase()}: ${File(it.path).name}" }
    }

    private fun saveNote() {
        val text = draftText.text.toString().ifBlank { null }
        val files = pending.toList()
        val locale = lastSettings().locale ?: NoteLocale.EN
        thread {
            val problem = try {
                core.addPhoneNote(text, locale, files)
                null
            } catch (e: org.openmyglass.corebindings.CoreFailure) {
                e.message
            }
            main.post {
                if (problem == null) {
                    files.forEach { File(it.path).delete() }
                    pending.clear()
                    draftText.setText("")
                    showDraftFiles()
                    refreshNotes()
                } else {
                    lastProblem = problem
                    refreshStatus()
                }
            }
        }
    }

    private fun refreshNotes() {
        notesBox.removeAllViews()
        val notes = core.notes()
        if (notes.isEmpty()) {
            notesBox.addView(TextView(this).apply { text = "None yet." })
        }
        notes.forEach { notesBox.addView(noteView(it)) }
    }

    private fun noteView(n: NoteInfo): View {
        val box = column().apply { setPadding(0, dp(8), 0, dp(8)); contentDescription = "note ${n.id}" }
        val date = DateFormat.getDateTimeInstance().format(Date(n.createdAtMs.toLong()))
        val from = if (n.origin == NoteOrigin.GLASS) "from the Glass" else "on this phone"
        val state = if (n.complete) "" else ", still arriving"
        box.addView(TextView(this).apply { text = "$date, $from$state"; textSize = 12f })
        n.text?.let { text -> box.addView(TextView(this).apply { this.text = text }) }
        n.transcript?.let { text -> box.addView(TextView(this).apply { this.text = "Heard: $text" }) }
        n.transcriptError?.let { why ->
            box.addView(TextView(this).apply { text = "No transcript: $why"; textSize = 12f })
        }
        n.attachments.forEach { box.addView(attachmentView(it)) }
        return box
    }

    private fun attachmentView(a: NoteAttachmentInfo): View {
        val path = a.path
        val mb = "%.1f MB".format(a.size.toLong() / 1_000_000.0)
        if (a.kind == NoteAttachmentKind.PHOTO && path != null) {
            val options = BitmapFactory.Options().apply { inSampleSize = 8 }
            BitmapFactory.decodeFile(path, options)?.let { bitmap ->
                return ImageView(this).apply {
                    setImageBitmap(bitmap)
                    adjustViewBounds = true
                    maxHeight = dp(160)
                    contentDescription = "photo"
                }
            }
        }
        val label = "${a.kind.name.lowercase()}, $mb" + if (path == null) " (arriving)" else ""
        if (a.kind == NoteAttachmentKind.VOICE && path != null) {
            return button("Play $label") { play(path) }
        }
        return TextView(this).apply { text = label }
    }

    private fun play(path: String) {
        player?.release()
        player = MediaPlayer().apply {
            setDataSource(path)
            prepare()
            start()
        }
    }

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
        val LOCALE_NAMES = mapOf(
            NoteLocale.IT to "Italiano",
            NoteLocale.EN to "English",
            NoteLocale.FR to "Français",
            NoteLocale.ES to "Español",
        )
    }
}
