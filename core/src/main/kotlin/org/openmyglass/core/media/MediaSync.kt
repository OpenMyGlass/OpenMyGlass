package org.openmyglass.core.media

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.openmyglass.protocol.v1.Bridge
import org.openmyglass.protocol.v1.Bridge.Frame

data class MediaItem(val id: String, val name: String, val video: Boolean, val size: Long, val takenAt: Long)

data class MediaServerInfo(val ip: String, val port: Int, val token: String)

sealed interface DownloadResult {
    data object Saved : DownloadResult
    data class Failed(val reason: String) : DownloadResult
}

/** Downloads one item from the Glass media server into the gallery, verified; never throws. */
fun interface MediaDownloader {
    suspend fun download(server: MediaServerInfo, item: MediaItem): DownloadResult
}

/** Resumable partial files for Bluetooth pulls; [complete] verifies the SHA-256 and saves to the gallery. */
interface PartialStore {
    fun length(id: String): Long
    fun append(id: String, bytes: ByteArray)
    fun reset(id: String)
    fun complete(item: MediaItem, sha256: String): DownloadResult
}

/** Ids already in the phone gallery. */
interface SyncedStore {
    fun contains(id: String): Boolean
    fun add(id: String)
}

/**
 * Phone side of photo/video sync (architecture §5.4). Automatic sync runs only while Glass is charging and on
 * Wi-Fi and the phone is on Wi-Fi ([autoEnabled] by the owner); [syncNow] runs on demand. A run lists Glass's
 * media, pulls the unsynced ones oldest first, one at a time, and closes the Glass server. A failed item is
 * retried on the next run; an automatic run stops after the current item when its conditions end.
 */
class MediaSync(
    private val synced: SyncedStore,
    private val downloader: MediaDownloader,
    private val autoEnabled: () -> Boolean,
    private val phoneOnWifi: () -> Boolean,
    private val scope: CoroutineScope,
    /** Bluetooth fallback (on demand only); null disables it. */
    private val partials: PartialStore? = null,
    /** Owner setting: delete on Glass after the phone saved and verified the file. */
    private val deleteAfterSync: () -> Boolean = { false },
    /** Glass hashes the file before its last chunk: allow for long videos (as the Wi-Fi path does). */
    private val chunkTimeoutMs: Long = 120_000,
    /** Glass must answer a list or pull request within this, or the run ends ("Glass did not answer"). */
    private val replyTimeoutMs: Long = 30_000,
    /** After an automatic run with failures or new media, one more run this much later if conditions hold. */
    private val followUpDelayMs: Long = 300_000,
) {
    companion object {
        /** Chunk payload over the bridge: a frame stays below the 64 KiB cap. */
        const val CHUNK_BYTES = 48 * 1024
    }

    private enum class Phase { IDLE, LISTING, WAITING_SERVER, DOWNLOADING }

    private val lock = Mutex()
    private var send: (suspend (Frame) -> Unit)? = null
    private var glass: Bridge.GlassStatus? = null
    private var phase = Phase.IDLE
    private var manual = false
    private var todo: List<MediaItem> = emptyList()
    /** Bumped on every attach/detach: a download running for an older session must not touch the new one. */
    private var generation = 0
    private var bluetooth = false
    /** Bumped on every phase change: a reply watchdog only fires for the wait it was armed for. */
    private var waitStamp = 0
    private var newMediaDuringRun = false
    @Volatile private var pendingChunk: kotlinx.coroutines.CompletableDeferred<Bridge.MediaChunk>? = null
    @Volatile private var pendingKey: Pair<String, Long>? = null

    private val _status = MutableStateFlow("Idle")
    /** One line for the phone UI. */
    val status: StateFlow<String> = _status.asStateFlow()

    suspend fun attach(send: suspend (Frame) -> Unit) = lock.withLock {
        this.send = send
        generation++
        phase = Phase.IDLE
        abandonChunk()
    }

    suspend fun detach() = lock.withLock {
        send = null
        glass = null
        generation++
        phase = Phase.IDLE
        abandonChunk()
    }

    /** A chunk request in flight will never be answered on a gone session: fail it now, not at the timeout. */
    private fun abandonChunk() {
        pendingChunk?.complete(Bridge.MediaChunk.newBuilder().setError("link lost").build())
        pendingChunk = null
    }

    /** The phone's Wi-Fi or the owner's auto-sync setting changed: start a run if everything now allows it. */
    suspend fun conditionsChanged() = lock.withLock {
        if (phase == Phase.IDLE && autoAllowed()) {
            manual = false
            requestList()
        }
    }

    /**
     * On demand, whatever the charging state: lists and pulls everything not yet synced, over Wi-Fi, or over the
     * Bluetooth bridge when [overBluetooth] (slow; the owner confirmed).
     */
    suspend fun syncNow(overBluetooth: Boolean = false) = lock.withLock {
        if (phase != Phase.IDLE) return@withLock
        manual = true
        bluetooth = overBluetooth && partials != null
        requestList()
    }

    suspend fun onFrame(frame: Frame) {
        when {
            // Only the answer to the request in flight: a late answer to an earlier (timed-out) request is dropped
            frame.hasMediaChunk() -> if (pendingKey == frame.mediaChunk.id to frame.mediaChunk.offset) pendingChunk?.complete(frame.mediaChunk)
            frame.hasGlassStatus() -> lock.withLock {
                glass = frame.glassStatus
                if (phase == Phase.IDLE && autoAllowed()) {
                    manual = false
                    bluetooth = false
                    requestList()
                }
            }
            frame.hasMediaList() -> lock.withLock {
                val items = frame.mediaList.itemsList.map { MediaItem(it.id, it.name, it.video, it.size, it.takenAtMs) }
                if (phase == Phase.DOWNLOADING) newMediaDuringRun = true
                val wanted = phase == Phase.LISTING || (phase == Phase.IDLE && autoAllowed())
                if (!wanted) return@withLock
                waitStamp++
                if (phase == Phase.IDLE) { manual = false; bluetooth = false }
                todo = items.filterNot { synced.contains(it.id) }.sortedBy { it.takenAt }
                if (todo.isEmpty()) {
                    phase = Phase.IDLE
                    _status.value = "Up to date"
                } else if (bluetooth) {
                    phase = Phase.DOWNLOADING
                    newMediaDuringRun = false
                    val items2 = todo
                    val gen = generation
                    scope.launch { download(null, items2, gen) }
                } else {
                    phase = Phase.WAITING_SERVER
                    _status.value = "${todo.size} to sync"
                    safeSend(Frame.newBuilder().setMediaPullStart(Bridge.MediaPullStart.getDefaultInstance()).build())
                    armReplyWatchdog()
                }
            }
            frame.hasMediaServerUnavailable() -> lock.withLock {
                waitStamp++
                if (phase == Phase.WAITING_SERVER) phase = Phase.IDLE
                _status.value = frame.mediaServerUnavailable.reason
            }
            frame.hasMediaServer() -> {
                val (items, gen) = lock.withLock {
                    if (phase != Phase.WAITING_SERVER) return
                    waitStamp++
                    newMediaDuringRun = false
                    phase = Phase.DOWNLOADING
                    todo to generation
                }
                val server = MediaServerInfo(frame.mediaServer.ip, frame.mediaServer.port, frame.mediaServer.token)
                scope.launch { download(server, items, gen) }
            }
        }
    }

    /** [server] null: Bluetooth chunks over the bridge. */
    private suspend fun download(server: MediaServerInfo?, items: List<MediaItem>, gen: Int) {
        var done = 0
        var failed = 0
        for (item in items) {
            val proceed = lock.withLock { gen == generation && send != null && (manual || autoAllowed()) }
            if (!proceed) break
            _status.value = "Syncing ${done + failed + 1} of ${items.size}"
            val result = try {
                if (server != null) downloader.download(server, item) else pullOverBridge(item, gen)
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                DownloadResult.Failed(e.message ?: "download failed")
            }
            when (result) {
                DownloadResult.Saved -> {
                    synced.add(item.id)
                    done++
                    if (deleteAfterSync()) lock.withLock {
                        if (gen == generation) safeSend(Frame.newBuilder().setMediaDelete(Bridge.MediaDelete.newBuilder().setId(item.id)).build())
                    }
                }
                is DownloadResult.Failed -> failed++
            }
        }
        lock.withLock {
            if (gen != generation) {   // a newer session owns the state and the Glass server now
                if (phase == Phase.IDLE) _status.value = "Sync interrupted (link lost), $failed failed, retried next time"
                return
            }
            if (server != null) safeSend(Frame.newBuilder().setMediaPullDone(Bridge.MediaPullDone.getDefaultInstance()).build())
            phase = Phase.IDLE
            // Failed items and media taken meanwhile get one more automatic try (issue #66)
            if (!manual && (failed > 0 || newMediaDuringRun)) {
                scope.launch {
                    kotlinx.coroutines.delay(followUpDelayMs)
                    if (lock.withLock { gen == generation }) conditionsChanged()
                }
            }
        }
        _status.value = if (failed == 0) "Synced $done" else "Synced $done, $failed failed (retried next time)"
    }

    /** Pulls one item in chunks, resuming from the partial file; the store verifies the hash and saves. */
    private suspend fun pullOverBridge(item: MediaItem, gen: Int): DownloadResult {
        val store = partials ?: return DownloadResult.Failed("Bluetooth sync unavailable")
        var offset = store.length(item.id)
        while (true) {
            val answer = kotlinx.coroutines.CompletableDeferred<Bridge.MediaChunk>()
            pendingKey = item.id to offset
            pendingChunk = answer
            val sentOk = lock.withLock {
                if (gen != generation || send == null) false
                else { safeSend(Frame.newBuilder().setMediaChunkRequest(Bridge.MediaChunkRequest.newBuilder()
                    .setId(item.id).setOffset(offset).setLength(CHUNK_BYTES)).build()); true }
            }
            if (!sentOk) return DownloadResult.Failed("link lost")
            val chunk = kotlinx.coroutines.withTimeoutOrNull(chunkTimeoutMs) { answer.await() }
                ?: return DownloadResult.Failed("Glass did not answer")   // the partial file is kept for resuming
            pendingChunk = null
            // A partial longer than the file (it changed, or two writers): start over next time
            if (chunk.total > 0 && offset > chunk.total) { store.reset(item.id); return DownloadResult.Failed("file changed on Glass") }
            if (chunk.error.isNotEmpty()) return DownloadResult.Failed(chunk.error)
            if (chunk.data.isEmpty) {
                return if (offset == chunk.total) store.complete(item, chunk.sha256) else DownloadResult.Failed("short chunk")
            }
            store.append(item.id, chunk.data.toByteArray())
            offset += chunk.data.size()
            if (offset == chunk.total) return store.complete(item, chunk.sha256)
            _status.value = "Bluetooth: ${item.name} ${offset * 100 / chunk.total}%"
        }
    }

    /** Caller holds the lock. A broken link must not crash the app; the session ends through the bridge anyway. */
    private suspend fun safeSend(frame: Frame) {
        val s = send ?: return
        try {
            s(frame)
        } catch (e: kotlinx.coroutines.CancellationException) {
            throw e
        } catch (_: Exception) {
        }
    }

    /** Caller holds the lock. */
    private fun autoAllowed(): Boolean {
        val g = glass ?: return false
        return autoEnabled() && g.charging && g.wifi && phoneOnWifi()
    }

    /** Caller holds the lock. */
    private suspend fun requestList() {
        if (send == null) return
        phase = Phase.LISTING
        _status.value = "Listing Glass media"
        safeSend(Frame.newBuilder().setMediaListRequest(Bridge.MediaListRequest.getDefaultInstance()).build())
        armReplyWatchdog()
    }

    /** Caller holds the lock. Ends a LISTING/WAITING_SERVER wait that Glass never answers (issue #64). */
    private fun armReplyWatchdog() {
        val stamp = ++waitStamp
        val gen = generation
        scope.launch {
            kotlinx.coroutines.delay(replyTimeoutMs)
            lock.withLock {
                if (stamp == waitStamp && gen == generation && (phase == Phase.LISTING || phase == Phase.WAITING_SERVER)) {
                    phase = Phase.IDLE
                    _status.value = "Glass did not answer"
                }
            }
        }
    }
}
