package org.openmyglass.core.dictation

/** An offline speech recognizer (architecture §5.3: the phone transcribes, never a cloud service). */
fun interface SpeechToText {
    /** Starts one utterance of 16-bit little-endian mono PCM at [sampleRate]. */
    fun start(sampleRate: Int): Recognition
}

interface Recognition {
    fun feed(pcm: ByteArray)
    /** The final text (may be empty); the recognition is done afterwards. */
    fun finish(): String
    fun cancel()
}

/** No usable recognizer right now (e.g. the speech model is not downloaded); the message is shown on Glass. */
class SpeechUnavailable(message: String) : Exception(message)

data class ReplyOutcome(val ok: Boolean, val message: String)
