package org.openmyglass.android.media

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import org.openmyglass.core.media.DownloadResult
import org.openmyglass.core.media.MediaDownloader
import org.openmyglass.core.media.MediaItem
import org.openmyglass.core.media.MediaServerInfo
import org.openmyglass.core.media.SyncedStore
import java.io.File

/** Owner choice: sync automatically while Glass charges on Wi-Fi (architecture §5.4). */
class MediaSettings(context: Context) {
    private val prefs = context.getSharedPreferences("media", Context.MODE_PRIVATE)

    var autoSync: Boolean
        get() = prefs.getBoolean("auto_sync", true)
        set(value) = prefs.edit().putBoolean("auto_sync", value).apply()
}

/** Ids already saved to the gallery. */
class SyncedMedia(context: Context) : SyncedStore {
    private val prefs = context.getSharedPreferences("media_synced", Context.MODE_PRIVATE)

    override fun contains(id: String) = prefs.getBoolean(id, false)

    override fun add(id: String) = prefs.edit().putBoolean(id, true).apply()
}

/** Pulls into a resumable partial file in the cache, then saves it to the gallery. */
class AndroidMediaDownloader(private val context: Context) : MediaDownloader {
    override suspend fun download(server: MediaServerInfo, item: MediaItem): DownloadResult =
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

object PhoneNetwork {
    /** The phone is on Wi-Fi (the Glass media server is reachable on the LAN only). */
    fun onWifi(context: Context): Boolean {
        val cm = context.getSystemService(ConnectivityManager::class.java) ?: return false
        return cm.getNetworkCapabilities(cm.activeNetwork)?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true
    }
}
