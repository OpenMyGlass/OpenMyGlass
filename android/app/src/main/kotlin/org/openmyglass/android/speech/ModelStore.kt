package org.openmyglass.android.speech

import android.content.Context
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL

/**
 * Speech models in app storage. Downloading is the only network use of dictation and happens only when the
 * owner asks for it in settings; the archive's SHA-256 is checked before it is unpacked.
 */
class ModelStore(context: Context) {
    private val root = File(context.filesDir, "speech-models")
    private val cache = context.cacheDir

    fun directory(model: SpeechModel) = File(root, model.name)

    fun isInstalled(model: SpeechModel) = File(directory(model), READY_MARK).isFile

    /** Blocking; run off the main thread. [progress] gets downloaded/total bytes. */
    fun download(model: SpeechModel, progress: (Long, Long) -> Unit = { _, _ -> }) {
        val zip = File(cache, "${model.name}.zip")
        val connection = URL(model.url).openConnection() as HttpURLConnection
        try {
            connection.connectTimeout = 15_000
            connection.readTimeout = 30_000
            if (connection.responseCode != 200) throw IOException("download failed: HTTP ${connection.responseCode}")
            val total = connection.contentLengthLong
            connection.inputStream.use { input ->
                zip.outputStream().use { out ->
                    val buf = ByteArray(64 * 1024)
                    var done = 0L
                    while (true) {
                        val n = input.read(buf)
                        if (n < 0) break
                        out.write(buf, 0, n)
                        done += n
                        progress(done, total)
                    }
                }
            }
        } finally {
            connection.disconnect()
        }
        try {
            if (ModelFiles.sha256(zip) != model.sha256) throw IOException("the downloaded model is corrupted (checksum mismatch)")
            val staging = File(root, "${model.name}.part").apply { deleteRecursively(); mkdirs() }
            zip.inputStream().use { ModelFiles.unzip(it, staging) }
            File(staging, READY_MARK).writeText(model.sha256)
            val dir = directory(model)
            dir.deleteRecursively()
            if (!staging.renameTo(dir)) throw IOException("cannot install the model")
        } finally {
            zip.delete()
        }
    }

    private companion object {
        const val READY_MARK = ".openmyglass-ready"
    }
}
