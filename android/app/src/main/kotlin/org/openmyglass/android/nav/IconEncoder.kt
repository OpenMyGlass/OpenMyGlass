package org.openmyglass.android.nav

import android.graphics.Bitmap
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.security.MessageDigest

/** Maneuver icons for Glass: scaled PNG (≤ 128 px, ≤ 16 KiB) keyed by the SHA-256 of the original pixels. */
object IconEncoder {
    const val MAX_SIDE = 128
    const val MAX_PNG_BYTES = 16 * 1024

    /** Fits (w, h) into a [max] × [max] box keeping the aspect ratio; never upscales. */
    fun scaleTo(w: Int, h: Int, max: Int = MAX_SIDE): Pair<Int, Int> {
        if (w <= max && h <= max) return w to h
        val scale = max.toDouble() / maxOf(w, h)
        return maxOf(1, (w * scale).toInt()) to maxOf(1, (h * scale).toInt())
    }

    /** SHA-256 (hex) of the bitmap's pixels, as in the capture fixtures and MapsIcons. */
    fun pixelSha256(bitmap: Bitmap): String {
        val readable = if (bitmap.config == Bitmap.Config.HARDWARE) bitmap.copy(Bitmap.Config.ARGB_8888, false) else bitmap
        val buffer = ByteBuffer.allocate(readable.byteCount).also { readable.copyPixelsToBuffer(it) }
        return MessageDigest.getInstance("SHA-256").digest(buffer.array()).joinToString("") { "%02x".format(it) }
    }

    /** (pixel SHA-256 hex, PNG bytes), or null if the bitmap cannot be read or the PNG is too large. */
    fun encode(bitmap: Bitmap): Pair<String, ByteArray>? = runCatching {
        val sha = pixelSha256(bitmap)
        val (w, h) = scaleTo(bitmap.width, bitmap.height)
        val readable = if (bitmap.config == Bitmap.Config.HARDWARE) bitmap.copy(Bitmap.Config.ARGB_8888, false) else bitmap
        val scaled = if (w == bitmap.width && h == bitmap.height) readable else Bitmap.createScaledBitmap(readable, w, h, true)
        val png = ByteArrayOutputStream().also { scaled.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        if (png.size > MAX_PNG_BYTES) null else sha to png
    }.getOrNull()
}
