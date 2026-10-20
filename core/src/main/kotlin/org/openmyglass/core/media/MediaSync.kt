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
) {
    private enum class Phase { IDLE, LISTING, WAITING_SERVER, DOWNLOADING }

    private val lock = Mutex()
    private var send: (suspend (Frame) -> Unit)? = null
    private var glass: Bridge.GlassStatus? = null
    private var phase = Phase.IDLE
    private var manual = false
    private var todo: List<MediaItem> = emptyList()
    /** Bumped on every attach/detach: a download running for an older session must not touch the new one. */
    private var generation = 0

    private val _status = MutableStateFlow("Idle")
    /** One line for the phone UI. */
    val status: StateFlow<String> = _status.asStateFlow()

    suspend fun attach(send: suspend (Frame) -> Unit) = lock.withLock {
        this.send = send
        generation++
        phase = Phase.IDLE
    }

    suspend fun detach() = lock.withLock {
        send = null
        glass = null
        generation++
        phase = Phase.IDLE
    }

    /** The phone's Wi-Fi or the owner's auto-sync setting changed: start a run if everything now allows it. */
    suspend fun conditionsChanged() = lock.withLock {
        if (phase == Phase.IDLE && autoAllowed()) {
            manual = false
            requestList()
        }
    }

    /** On demand, whatever the charging state: lists and pulls everything not yet synced. */
    suspend fun syncNow() = lock.withLock {
        if (phase != Phase.IDLE) return@withLock
        manual = true
        requestList()
    }

    suspend fun onFrame(frame: Frame) {
        when {
            frame.hasGlassStatus() -> lock.withLock {
                glass = frame.glassStatus
                if (phase == Phase.IDLE && autoAllowed()) {
                    manual = false
                    requestList()
                }
            }
            frame.hasMediaList() -> lock.withLock {
                val items = frame.mediaList.itemsList.map { MediaItem(it.id, it.name, it.video, it.size, it.takenAtMs) }
                val wanted = phase == Phase.LISTING || (phase == Phase.IDLE && autoAllowed())
                if (!wanted) return@withLock
                if (phase == Phase.IDLE) manual = false
                todo = items.filterNot { synced.contains(it.id) }.sortedBy { it.takenAt }
                if (todo.isEmpty()) {
                    phase = Phase.IDLE
                    _status.value = "Up to date"
                } else {
                    phase = Phase.WAITING_SERVER
                    _status.value = "${todo.size} to sync"
                    safeSend(Frame.newBuilder().setMediaPullStart(Bridge.MediaPullStart.getDefaultInstance()).build())
                }
            }
            frame.hasMediaServerUnavailable() -> lock.withLock {
                if (phase == Phase.WAITING_SERVER) phase = Phase.IDLE
                _status.value = frame.mediaServerUnavailable.reason
            }
            frame.hasMediaServer() -> {
                val (items, gen) = lock.withLock {
                    if (phase != Phase.WAITING_SERVER) return
                    phase = Phase.DOWNLOADING
                    todo to generation
                }
                val server = MediaServerInfo(frame.mediaServer.ip, frame.mediaServer.port, frame.mediaServer.token)
                scope.launch { download(server, items, gen) }
            }
        }
    }

    private suspend fun download(server: MediaServerInfo, items: List<MediaItem>, gen: Int) {
        var done = 0
        var failed = 0
        for (item in items) {
            val proceed = lock.withLock { gen == generation && send != null && (manual || autoAllowed()) }
            if (!proceed) break
            _status.value = "Syncing ${done + failed + 1} of ${items.size}"
            val result = try {
                downloader.download(server, item)
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                DownloadResult.Failed(e.message ?: "download failed")
            }
            when (result) {
                DownloadResult.Saved -> { synced.add(item.id); done++ }
                is DownloadResult.Failed -> failed++
            }
        }
        lock.withLock {
            if (gen != generation) return   // a newer session owns the state and the Glass server now
            safeSend(Frame.newBuilder().setMediaPullDone(Bridge.MediaPullDone.getDefaultInstance()).build())
            phase = Phase.IDLE
        }
        _status.value = if (failed == 0) "Synced $done" else "Synced $done, $failed failed (retried next time)"
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
    }
}
