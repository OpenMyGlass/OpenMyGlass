package org.openmyglass.companion

import android.content.Context
import android.content.Intent
import android.media.AudioFormat
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import org.openmyglass.corebindings.NoteLocale
import org.openmyglass.corebindings.SpeechAudio
import org.openmyglass.corebindings.SpeechFailure
import org.openmyglass.corebindings.SpeechToTextEngine
import org.openmyglass.corebindings.TextToSpeechEngine
import java.io.File
import java.util.Locale
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

private fun NoteLocale.toJava(): Locale = when (this) {
    NoteLocale.IT -> Locale.ITALIAN
    NoteLocale.EN -> Locale.ENGLISH
    NoteLocale.FR -> Locale.FRENCH
    NoteLocale.ES -> Locale("es")
}

/**
 * The platform's text-to-speech. Speech always runs here on the companion, never on the Glass.
 * The core calls this on its own thread, so waiting is fine.
 */
class AndroidTts(private val context: Context) : TextToSpeechEngine {
    override fun synthesize(text: String, locale: NoteLocale): SpeechAudio {
        val ready = CountDownLatch(1)
        val status = AtomicReference(TextToSpeech.ERROR)
        val tts = TextToSpeech(context) { s ->
            status.set(s)
            ready.countDown()
        }
        try {
            if (!ready.await(10, TimeUnit.SECONDS) || status.get() != TextToSpeech.SUCCESS) {
                throw SpeechFailure.Unavailable()
            }
            if (tts.isLanguageAvailable(locale.toJava()) < TextToSpeech.LANG_AVAILABLE) {
                throw SpeechFailure.Unavailable()
            }
            tts.language = locale.toJava()
            return synthesizeToPcm(tts, text)
        } finally {
            tts.shutdown()
        }
    }

    private fun synthesizeToPcm(tts: TextToSpeech, text: String): SpeechAudio {
        val file = File.createTempFile("tts", ".wav", context.cacheDir)
        try {
            val done = CountDownLatch(1)
            val error = AtomicReference<String?>(null)
            tts.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
                override fun onStart(id: String?) = Unit
                override fun onDone(id: String?) = done.countDown()

                @Deprecated("Deprecated in Android")
                override fun onError(id: String?) {
                    error.set("the engine reported an error")
                    done.countDown()
                }

                override fun onError(id: String?, code: Int) {
                    error.set("the engine reported error $code")
                    done.countDown()
                }
            })
            if (tts.synthesizeToFile(text, Bundle(), file, "note") != TextToSpeech.SUCCESS) {
                throw SpeechFailure.Failed("the engine refused the text")
            }
            if (!done.await(60, TimeUnit.SECONDS)) throw SpeechFailure.Failed("timed out")
            error.get()?.let { throw SpeechFailure.Failed(it) }
            val audio = Wav.parse(file.readBytes())
                ?: throw SpeechFailure.Failed("the engine did not produce PCM")
            return SpeechAudio(audio.sampleRate.toUInt(), audio.samples)
        } finally {
            file.delete()
        }
    }
}

/**
 * Transcribes a WAV recording with the platform's on-device recognizer, fed from the file.
 * This needs Android 13 and a recognizer that supports it: anything else is "no engine", which
 * the note then shows. A note's text from speech is never made on the Glass.
 */
class AndroidStt(private val context: Context) : SpeechToTextEngine {
    override fun transcribe(audioPath: String, mime: String, locale: NoteLocale): String {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            !SpeechRecognizer.isOnDeviceRecognitionAvailable(context) ||
            !mime.contains("wav")
        ) {
            throw SpeechFailure.Unavailable()
        }
        val audio = Wav.parse(File(audioPath).readBytes()) ?: throw SpeechFailure.Unavailable()
        val pipe = ParcelFileDescriptor.createPipe()
        val writer = Thread {
            ParcelFileDescriptor.AutoCloseOutputStream(pipe[1]).use { it.write(audio.samples) }
        }
        val result = AtomicReference<Result<String>?>(null)
        val done = CountDownLatch(1)
        val main = Handler(Looper.getMainLooper())
        var recognizer: SpeechRecognizer? = null
        main.post {
            recognizer = SpeechRecognizer.createOnDeviceSpeechRecognizer(context).apply {
                setRecognitionListener(listener(result, done))
                startListening(
                    Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
                        .putExtra(
                            RecognizerIntent.EXTRA_LANGUAGE_MODEL,
                            RecognizerIntent.LANGUAGE_MODEL_FREE_FORM,
                        )
                        .putExtra(RecognizerIntent.EXTRA_LANGUAGE, locale.toJava().toLanguageTag())
                        .putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE, pipe[0])
                        .putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_CHANNEL_COUNT, 1)
                        .putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_ENCODING, AudioFormat.ENCODING_PCM_16BIT)
                        .putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_SAMPLING_RATE, audio.sampleRate),
                )
            }
            writer.start()
        }
        val answered = done.await(120, TimeUnit.SECONDS)
        main.post { recognizer?.destroy() }
        pipe[0].close()
        if (!answered) throw SpeechFailure.Failed("timed out")
        return result.get()?.getOrThrow() ?: throw SpeechFailure.Failed("no answer")
    }

    private fun listener(result: AtomicReference<Result<String>?>, done: CountDownLatch) =
        object : RecognitionListener {
            override fun onResults(results: Bundle?) {
                val text = results?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull()
                result.set(Result.success(text.orEmpty()))
                done.countDown()
            }

            override fun onError(error: Int) {
                val failure = when (error) {
                    SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED,
                    SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE,
                    SpeechRecognizer.ERROR_RECOGNIZER_BUSY,
                    SpeechRecognizer.ERROR_CLIENT,
                    -> SpeechFailure.Unavailable()
                    else -> SpeechFailure.Failed("recognizer error $error")
                }
                result.set(Result.failure(failure))
                done.countDown()
            }

            override fun onReadyForSpeech(params: Bundle?) = Unit
            override fun onBeginningOfSpeech() = Unit
            override fun onRmsChanged(rmsdB: Float) = Unit
            override fun onBufferReceived(buffer: ByteArray?) = Unit
            override fun onEndOfSpeech() = Unit
            override fun onPartialResults(partialResults: Bundle?) = Unit
            override fun onEvent(eventType: Int, params: Bundle?) = Unit
        }
}
