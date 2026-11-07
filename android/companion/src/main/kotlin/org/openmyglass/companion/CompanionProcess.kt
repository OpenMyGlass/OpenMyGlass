package org.openmyglass.companion

import android.content.Context
import android.os.Handler
import android.os.Looper
import org.openmyglass.corebindings.CompanionCore
import org.openmyglass.corebindings.CoreEvent
import org.openmyglass.corebindings.NewAttachment
import java.io.File
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import kotlin.concurrent.thread

/** What a screen is told. Always on the main thread, and only while it is [CompanionProcess.screen]. */
interface CompanionScreen {
    /** [bleAddress] is that of the BLE link the events came from, if they came from one. */
    fun onCoreEvents(events: List<CoreEvent>, bleAddress: String?)
    fun onLinkFailed(why: String)
    fun onDraftChanged(problem: String?)
}

/**
 * Polls the core until the process ends. A panic in the core reaches Kotlin as an exception, as
 * uniffi's `InternalException`, which is not a `RuntimeException`. It is reported once, and polling
 * slows down until a poll works again, rather than the process going with it.
 */
internal class Poller(
    private val poll: () -> List<CoreEvent>,
    private val deliver: (List<CoreEvent>) -> Unit,
    private val report: (String) -> Unit,
    private val sleep: (Long) -> Unit = { Thread.sleep(it) },
) {
    /** How long the next round waits first. */
    var waitMs = POLL_INTERVAL_MS
        private set

    fun run() {
        while (true) step()
    }

    /** One round: wait, then poll. */
    fun step() {
        sleep(waitMs)
        try {
            val events = poll()
            if (events.isNotEmpty()) deliver(events)
            waitMs = POLL_INTERVAL_MS
        } catch (e: Exception) {
            if (waitMs == POLL_INTERVAL_MS) report("The core failed, and keeps trying: ${e.message ?: e}")
            waitMs = (waitMs * 2).coerceAtMost(MAX_POLL_WAIT_MS)
        }
    }

    companion object {
        const val POLL_INTERVAL_MS = 500L
        const val MAX_POLL_WAIT_MS = 60_000L
    }
}

/** A note being written. It lives as long as the process, whichever screen shows it. */
class NoteDraft {
    var text = ""
    val files = mutableListOf<NewAttachment>()
    var saving = false
}

/**
 * What must outlive any one screen, one per process: the core, the link and the draft. Android can
 * recreate or destroy an activity whenever it likes, and none of that should end a link, lose a
 * draft or open a second core on the same directory.
 */
class CompanionProcess private constructor(context: Context) {
    private val main = Handler(Looper.getMainLooper())

    val core: CompanionCore
    val link: LinkController
    val draft = NoteDraft()

    /**
     * The one thread the screen calls the core on. A call waits for the core's lock, which can be
     * held for seconds while a big attachment is hashed, so none may run on the main thread.
     */
    val coreCalls: ExecutorService = Executors.newSingleThreadExecutor()

    /** Main thread only. */
    var screen: CompanionScreen? = null

    /** Main thread only. Whether a screen has tried to go back to the bonded Glass. */
    var triedBondedGlass = false

    /** Main thread only. What is going on while no link is up, set while a BLE link is wanted. */
    var linkNote: String? = null

    /** Main thread only. The last thing that went wrong with the link. */
    var lastProblem: String? = null

    /** Main thread only. Numbers requests to read a note aloud, across screens. */
    var lastSpeechRequest = 0uL

    init {
        // A draft does not outlive the process, so these copies are left from one that ended.
        context.cacheDir.listFiles()
            ?.filter { f -> CACHE_PREFIXES.any { f.name.startsWith(it) } }
            ?.forEach { it.delete() }
        core = CompanionCore(
            File(context.filesDir, "companion").path,
            android.os.Build.MODEL,
            context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: "0",
            AndroidStt(context),
            AndroidTts(context),
        )
        link = LinkController(
            core,
            { context.filesDir.usableSpace },
            onOpenFailed = { request, why ->
                // The owner may have asked for something else since.
                toScreen { if (link.wants(request)) it.onLinkFailed(why) }
            },
        ) { events, from ->
            // Read now: by the time the post runs, another link may be the current one.
            deliver(events, (from as? BleLink)?.address)
        }
        // A note can change with no link, when speech finishes, and nothing pumps then.
        thread(isDaemon = true, name = "omg-poll") { poll() }
    }

    private fun poll() = Poller(
        poll = { core.poll() },
        deliver = { deliver(it, null) },
        report = { why -> toScreen { it.onLinkFailed(why) } },
    ).run()

    private fun deliver(events: List<CoreEvent>, bleAddress: String?) =
        toScreen { it.onCoreEvents(events, bleAddress) }

    private fun toScreen(tell: (CompanionScreen) -> Unit) {
        main.post { screen?.let(tell) }
    }

    /** Saves the draft as it is now, on its own thread. Main thread only. */
    fun saveDraft(locale: org.openmyglass.corebindings.NoteLocale) {
        val files = draft.files.toList()
        val text = draft.text.ifBlank { null }
        draft.saving = true
        thread {
            val problem = try {
                core.addPhoneNote(text, locale, files)
                null
            } catch (e: org.openmyglass.corebindings.CoreFailure) {
                e.message ?: "the note could not be saved"
            }
            main.post {
                draft.saving = false
                if (problem == null) {
                    files.forEach { File(it.path).delete() }
                    draft.files.removeAll(files)
                    draft.text = ""
                }
                screen?.onDraftChanged(problem)
            }
        }
    }

    companion object {
        private val CACHE_PREFIXES = listOf("pick", "voice", "tts")

        private var instance: CompanionProcess? = null

        /**
         * The one for this process, made on the first call. Throws `CoreFailure` if the core
         * cannot open its state, a full disk for instance, and the next call tries again.
         */
        @Synchronized
        fun of(context: Context): CompanionProcess =
            instance ?: CompanionProcess(context.applicationContext).also { instance = it }
    }
}
