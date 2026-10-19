package org.openmyglass.android.speech

import java.io.File
import java.io.IOException
import java.io.InputStream
import java.security.MessageDigest
import java.util.zip.ZipInputStream

/** An offline speech model the owner can download once (Vosk small models, Apache 2.0). */
data class SpeechModel(val language: String, val name: String, val url: String, val sha256: String, val sizeMb: Int)

object ModelCatalog {
    val ITALIAN = SpeechModel("it", "vosk-model-small-it-0.22", "https://alphacephei.com/vosk/models/vosk-model-small-it-0.22.zip",
        "9ec65e75861d1c6c2e457cccd932705340dcdf233f5b239f00733b4de0bf3267", 48)
    val ENGLISH = SpeechModel("en", "vosk-model-small-en-us-0.15", "https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip",
        "30f26242c4eb449f948e42cb302dd7a686cb29a3423a8367f99ff41780942498", 40)

    fun forLanguage(language: String) = if (language == "en") ENGLISH else ITALIAN
}

object ModelFiles {
    fun sha256(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buf = ByteArray(64 * 1024)
            while (true) {
                val n = input.read(buf)
                if (n < 0) break
                digest.update(buf, 0, n)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    /** Extracts a model zip into [dest], dropping the archive's top folder; rejects entries that escape [dest]. */
    fun unzip(input: InputStream, dest: File) {
        val root = dest.canonicalFile
        ZipInputStream(input).use { zip ->
            while (true) {
                val entry = zip.nextEntry ?: break
                val relative = entry.name.substringAfter('/', "")
                if (relative.isEmpty()) continue
                val target = File(root, relative).canonicalFile
                if (!target.path.startsWith(root.path + File.separator)) throw IOException("unsafe zip entry: ${entry.name}")
                if (entry.isDirectory) {
                    target.mkdirs()
                } else {
                    target.parentFile?.mkdirs()
                    target.outputStream().use { zip.copyTo(it) }
                }
            }
        }
    }
}
