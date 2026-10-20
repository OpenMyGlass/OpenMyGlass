package org.openmyglass.android.media

import java.io.File
import java.io.FileOutputStream
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * Downloads one media file from the Glass media server into [partial], resuming with Range when part of it is
 * already there, and checks the server's SHA-256 over the whole file. A mismatch deletes the partial file (the
 * next attempt starts over); a broken transfer keeps it for resuming. Blocking.
 */
object HttpDownload {
    sealed interface Result {
        data object Ok : Result
        data class Failed(val reason: String) : Result
    }

    fun fetch(url: String, partial: File): Result {
        val have = if (partial.isFile) partial.length() else 0L
        val c = URL(url).openConnection() as HttpURLConnection
        return try {
            c.connectTimeout = 10_000
            // Glass hashes a file the first time it serves it before answering: allow for long videos
            c.readTimeout = 120_000
            if (have > 0) c.setRequestProperty("Range", "bytes=$have-")
            val code = c.responseCode
            val append = when (code) {
                206 -> true
                200 -> false   // the server ignored the range: start over
                416 -> { partial.delete(); return Result.Failed("stale partial file") }
                else -> return Result.Failed("HTTP $code")
            }
            val expected = c.getHeaderField("X-Content-SHA256") ?: return Result.Failed("no checksum")
            val total = (if (append) have else 0L) + c.contentLengthLong
            c.inputStream.use { input -> FileOutputStream(partial, append).use { input.copyTo(it, 64 * 1024) } }
            // A connection closed early can look like a normal end of stream: keep the part and resume next time
            if (c.contentLengthLong >= 0 && partial.length() < total) return Result.Failed("transfer interrupted")
            if (sha256(partial) != expected) {
                partial.delete()
                Result.Failed("checksum mismatch")
            } else Result.Ok
        } catch (e: java.io.IOException) {
            Result.Failed(e.message ?: "transfer interrupted")
        } finally {
            c.disconnect()
        }
    }

    private fun sha256(file: File): String {
        val d = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buf = ByteArray(64 * 1024)
            while (true) {
                val n = input.read(buf)
                if (n < 0) break
                d.update(buf, 0, n)
            }
        }
        return d.digest().joinToString("") { "%02x".format(it) }
    }
}
