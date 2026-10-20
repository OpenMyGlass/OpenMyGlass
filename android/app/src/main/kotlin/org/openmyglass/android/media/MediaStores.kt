package org.openmyglass.android.media

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import org.openmyglass.core.media.DownloadResult
import org.openmyglass.core.media.MediaDownloader
import org.openmyglass.core.media.MediaItem
import org.openmyglass.core.media.MediaServerInfo
import org.openmyglass.core.media.SyncedStore
import kotlinx.coroutines.sync.withLock
import java.io.File

/** Owner choice: sync automatically while Glass charges on Wi-Fi (architecture §5.4). */
class MediaSettings(context: Context) {
    private val prefs = context.getSharedPreferences("media", Context.MODE_PRIVATE)

    var autoSync: Boolean
        get() = prefs.getBoolean("auto_sync", true)
        set(value) = prefs.edit().putBoolean("auto_sync", value).apply()

    /** Delete on Glass once a file is saved and verified on the phone (default off). */
    var deleteAfterSync: Boolean
        get() = prefs.getBoolean("delete_after_sync", false)
        set(value) = prefs.edit().putBoolean("delete_after_sync", value).apply()
}

/** Ids already saved to the gallery. */
class SyncedMedia(context: Context) : SyncedStore {
    private val prefs = context.getSharedPreferences("media_synced", Context.MODE_PRIVATE)

    override fun contains(id: String) = prefs.getBoolean(id, false)

    override fun add(id: String) = prefs.edit().putBoolean(id, true).apply()
}

/** Pulls into a resumable partial file in the cache, then saves it to the gallery. */
class AndroidMediaDownloader(private val context: Context) : MediaDownloader {
    /** One download at a time: a late run from an old session must never write the same partial file. */
    private val oneAtATime = kotlinx.coroutines.sync.Mutex()

    override suspend fun download(server: MediaServerInfo, item: MediaItem): DownloadResult = oneAtATime.withLock {
        kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
            val dir = File(context.cacheDir, "media-partial").apply { mkdirs() }
            val partial = File(dir, item.id)
            when (val r = HttpDownload.fetch("http://${server.ip}:${server.port}/media/${item.id}?token=${server.token}", partial)) {
                is HttpDownload.Result.Failed -> DownloadResult.Failed(r.reason)
                HttpDownload.Result.Ok -> {
                    val saved = GallerySaver.save(context, item, partial)
                    partial.delete()
                    if (saved) DownloadResult.Saved else DownloadResult.Failed("gallery write failed")
                }
            }
        }
    }
}

object PhoneNetwork {
    /** The phone is on Wi-Fi (the Glass media server is reachable on the LAN only). */
    fun onWifi(context: Context): Boolean {
        val cm = context.getSystemService(ConnectivityManager::class.java) ?: return false
        return cm.getNetworkCapabilities(cm.activeNetwork)?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true
    }
}

/** Partial files for Bluetooth pulls, shared with the HTTP path (same names): a transfer resumes on either. */
class FilePartialStore(private val dir: File, private val save: (MediaItem, File) -> Boolean) : org.openmyglass.core.media.PartialStore {
    private fun file(id: String) = File(dir.apply { mkdirs() }, id)

    override fun length(id: String) = file(id).let { if (it.isFile) it.length() else 0L }

    override fun append(id: String, bytes: ByteArray) = java.io.FileOutputStream(file(id), true).use { it.write(bytes) }

    override fun reset(id: String) {
        file(id).delete()
    }

    override fun complete(item: MediaItem, sha256: String): DownloadResult {
        val f = file(item.id)
        if (!f.isFile) return DownloadResult.Failed("missing partial file")
        val actual = java.security.MessageDigest.getInstance("SHA-256").let { d ->
            f.inputStream().use { input ->
                val buf = ByteArray(64 * 1024)
                while (true) { val n = input.read(buf); if (n < 0) break; d.update(buf, 0, n) }
            }
            d.digest().joinToString("") { "%02x".format(it) }
        }
        return try {
            when {
                actual != sha256 -> DownloadResult.Failed("checksum mismatch")
                save(item, f) -> DownloadResult.Saved
                else -> DownloadResult.Failed("gallery write failed")
            }
        } finally {
            f.delete()
        }
    }
}
