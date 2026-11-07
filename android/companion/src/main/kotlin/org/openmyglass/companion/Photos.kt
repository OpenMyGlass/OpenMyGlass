package org.openmyglass.companion

import android.graphics.Bitmap
import android.graphics.ImageDecoder
import java.io.File
import java.io.IOException

object Photos {
    /** The power of two that brings the longer side of a photo down to at most [maxSide]. */
    fun sampleSize(width: Int, height: Int, maxSide: Int): Int {
        var sample = 1
        while (maxOf(width, height) / sample > maxSide) sample *= 2
        return sample
    }

    /** A photo no bigger than [maxSide], upright as its EXIF says. Null if it cannot be shown. */
    fun decode(path: String, maxSide: Int): Bitmap? = try {
        ImageDecoder.decodeBitmap(ImageDecoder.createSource(File(path))) { decoder, info, _ ->
            decoder.setTargetSampleSize(sampleSize(info.size.width, info.size.height, maxSide))
        }
    } catch (_: IOException) {
        null
    } catch (_: RuntimeException) {
        // A photo from outside this app can be anything, and none is worth the app.
        null
    } catch (_: OutOfMemoryError) {
        null
    }
}
