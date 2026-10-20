package org.openmyglass.android.media

import android.content.ContentValues
import android.content.Context
import android.provider.MediaStore
import org.openmyglass.core.media.MediaItem
import java.io.File

/** Puts a verified file into the gallery (DCIM/Glass), hidden until complete (IS_PENDING). */
object GallerySaver {
    fun save(context: Context, item: MediaItem, file: File): Boolean {
        val resolver = context.contentResolver
        val collection = if (item.video) MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        else MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, item.name)
            put(MediaStore.MediaColumns.MIME_TYPE, mimeType(item.name))
            put(MediaStore.MediaColumns.RELATIVE_PATH, "DCIM/Glass")
            put(MediaStore.MediaColumns.DATE_TAKEN, item.takenAt)
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val uri = try {
            resolver.insert(collection, values)
        } catch (e: Exception) {
            null
        } ?: return false
        return try {
            resolver.openOutputStream(uri)?.use { out -> file.inputStream().use { it.copyTo(out, 64 * 1024) } }
                ?: throw java.io.IOException("cannot open gallery item")
            resolver.update(uri, ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }, null, null)
            true
        } catch (e: Exception) {
            resolver.delete(uri, null, null)
            false
        }
    }

    private fun mimeType(name: String) = when (name.substringAfterLast('.').lowercase()) {
        "mp4" -> "video/mp4"
        "3gp" -> "video/3gpp"
        "png" -> "image/png"
        else -> "image/jpeg"
    }
}
