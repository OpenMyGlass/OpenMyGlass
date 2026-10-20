package org.openmyglass.android.media

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.openmyglass.core.media.DownloadResult
import org.openmyglass.core.media.MediaItem
import java.security.MessageDigest

class FilePartialStoreTest {
    @get:Rule val tmp = TemporaryFolder()
    private val item = MediaItem("v1", "v1.mp4", true, 6, 1)
    private val saved = mutableListOf<ByteArray>()
    private fun store(saveOk: Boolean = true) = FilePartialStore(tmp.root) { _, f -> saved += f.readBytes(); saveOk }
    private fun sha(b: ByteArray) = MessageDigest.getInstance("SHA-256").digest(b).joinToString("") { "%02x".format(it) }

    @Test
    fun `appends, resumes from its length and saves a verified file`() {
        val s = store()
        s.append("v1", byteArrayOf(1, 2, 3))
        assertEquals(3, store().length("v1"))          // survives a new instance (resume after restart)
        s.append("v1", byteArrayOf(4, 5, 6))
        assertEquals(DownloadResult.Saved, s.complete(item, sha(byteArrayOf(1, 2, 3, 4, 5, 6))))
        assertTrue(saved.single().contentEquals(byteArrayOf(1, 2, 3, 4, 5, 6)))
        assertEquals(0, s.length("v1"))
    }

    @Test
    fun `a wrong hash discards the partial and saves nothing`() {
        val s = store()
        s.append("v1", byteArrayOf(1, 2, 3))
        assertTrue(s.complete(item, "00".repeat(32)) is DownloadResult.Failed)
        assertTrue(saved.isEmpty()); assertEquals(0, s.length("v1"))
    }

    @Test
    fun `a failed gallery write keeps nothing marked`() {
        val s = store(saveOk = false)
        s.append("v1", byteArrayOf(9))
        assertTrue(s.complete(item, sha(byteArrayOf(9))) is DownloadResult.Failed)
        assertFalse(tmp.root.resolve("v1").exists())
    }
}
