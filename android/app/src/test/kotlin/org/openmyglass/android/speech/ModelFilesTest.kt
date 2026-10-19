package org.openmyglass.android.speech

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

class ModelFilesTest {
    @get:Rule val tmp = TemporaryFolder()

    private fun zip(vararg entries: Pair<String, String>): ByteArray = ByteArrayOutputStream().also { out ->
        ZipOutputStream(out).use { z -> entries.forEach { (name, body) -> z.putNextEntry(ZipEntry(name)); z.write(body.toByteArray()); z.closeEntry() } }
    }.toByteArray()

    @Test
    fun `catalog pins the italian and english small models`() {
        val it = ModelCatalog.forLanguage("it")
        assertEquals("vosk-model-small-it-0.22", it.name)
        assertEquals("9ec65e75861d1c6c2e457cccd932705340dcdf233f5b239f00733b4de0bf3267", it.sha256)
        assertTrue(it.url.startsWith("https://"))
        assertEquals("vosk-model-small-en-us-0.15", ModelCatalog.forLanguage("en").name)
    }

    @Test
    fun `sha256 of a file`() {
        val f = tmp.newFile().apply { writeText("abc") }
        assertEquals("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", ModelFiles.sha256(f))
    }

    @Test
    fun `unzip strips the top folder`() {
        val dest = tmp.newFolder("model")
        ModelFiles.unzip(ByteArrayInputStream(zip("vosk-model-small-it-0.22/am/final.mdl" to "x", "vosk-model-small-it-0.22/conf/model.conf" to "y")), dest)
        assertTrue(File(dest, "am/final.mdl").isFile)
        assertTrue(File(dest, "conf/model.conf").isFile)
    }

    @Test
    fun `unzip rejects entries escaping the folder`() {
        val dest = tmp.newFolder("model")
        try {
            ModelFiles.unzip(ByteArrayInputStream(zip("m/../../evil.txt" to "x")), dest)
            fail("zip slip accepted")
        } catch (_: IOException) { }
        assertFalse(File(tmp.root, "evil.txt").exists())
    }
}
