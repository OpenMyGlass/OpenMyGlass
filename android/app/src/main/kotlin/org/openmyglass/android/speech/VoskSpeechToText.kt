package org.openmyglass.android.speech

import org.json.JSONObject
import org.openmyglass.core.dictation.Recognition
import org.openmyglass.core.dictation.SpeechToText
import org.openmyglass.core.dictation.SpeechUnavailable
import org.vosk.Model
import org.vosk.Recognizer

/** Vosk, fully offline, with a model downloaded through [ModelStore]. The model is loaded once and kept. */
class VoskSpeechToText(private val store: ModelStore, private val model: () -> SpeechModel) : SpeechToText {
    private var loaded: Pair<SpeechModel, Model>? = null

    @Synchronized
    private fun load(): Model {
        val wanted = model()
        loaded?.let { (m, vosk) -> if (m == wanted) return vosk else vosk.close() }
        if (!store.isInstalled(wanted)) {
            val language = if (wanted.language == "en") "English" else "Italian"
            throw SpeechUnavailable("Download the $language speech model: OpenMyGlass settings on the phone")
        }
        return Model(store.directory(wanted).absolutePath).also { loaded = wanted to it }
    }

    override fun start(sampleRate: Int): Recognition {
        val recognizer = try {
            Recognizer(load(), sampleRate.toFloat())
        } catch (e: SpeechUnavailable) {
            throw e
        } catch (e: Exception) {
            throw SpeechUnavailable("Speech recognition failed to start")
        }
        return object : Recognition {
            override fun feed(pcm: ByteArray) {
                recognizer.acceptWaveForm(pcm, pcm.size)
            }

            override fun finish(): String = try {
                JSONObject(recognizer.finalResult).optString("text")
            } finally {
                recognizer.close()
            }

            override fun cancel() = recognizer.close()
        }
    }
}
