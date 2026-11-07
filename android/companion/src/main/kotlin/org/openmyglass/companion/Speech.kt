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
import java.io.IOException
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
 * The core calls [synthesize] on its one speech thread, so waiting is fine, and one engine serves
 * every piece.
 */
class AndroidTts(private val context: Context) : TextToSpeechEngine {
    private class Piece {
        val done = CountDownLatch(1)
        val error = AtomicReference<String?>(null)
    }

    @Volatile private var engine: TextToSpeech? = null

    @Volatile private var piece: Piece? = null

    override fun synthesize(text: String, locale: NoteLocale): SpeechAudio {
        val tts = engine ?: start()
        if (tts.isLanguageAvailable(locale.toJava()) < TextToSpeech.LANG_AVAILABLE) {
            throw SpeechFailure.Unavailable()
        }
        tts.language = locale.toJava()
        return synthesizeToPcm(tts, text)
    }

    override fun cancel() {
        engine?.stop()
        piece?.let {
            it.error.set("stopped")
            it.done.countDown()
        }
    }

    private fun start(): TextToSpeech {
        val ready = CountDownLatch(1)
        val status = AtomicReference(TextToSpeech.ERROR)
        val tts = TextToSpeech(context) { s ->
            status.set(s)
            ready.countDown()
        }
        if (!ready.await(10, TimeUnit.SECONDS) || status.get() != TextToSpeech.SUCCESS) {
            tts.shutdown()
            throw SpeechFailure.Unavailable()
        }
        engine = tts
        return tts
    }

    /** It may have died with its service, so the next piece starts another. */
    private fun forget(tts: TextToSpeech) {
        engine = null
        tts.shutdown()
    }

    private fun synthesizeToPcm(tts: TextToSpeech, text: String): SpeechAudio {
        val file = File.createTempFile("tts", ".wav", context.cacheDir)
        val current = Piece()
        piece = current
        try {
            tts.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
                override fun onStart(id: String?) = Unit
                override fun onDone(id: String?) = current.done.countDown()

                @Deprecated("Deprecated in Android")
                override fun onError(id: String?) {
                    current.error.compareAndSet(null, "the engine reported an error")
                    current.done.countDown()
                }

                override fun onError(id: String?, code: Int) {
                    current.error.compareAndSet(null, "the engine reported error $code")
                    current.done.countDown()
                }

                override fun onStop(id: String?, interrupted: Boolean) {
                    current.error.compareAndSet(null, "stopped")
                    current.done.countDown()
                }
            })
            if (tts.synthesizeToFile(text, Bundle(), file, "note") != TextToSpeech.SUCCESS) {
                forget(tts)
                throw SpeechFailure.Failed("the engine refused the text")
            }
            if (!current.done.await(60, TimeUnit.SECONDS)) {
                forget(tts)
                throw SpeechFailure.Failed("timed out")
            }
            current.error.get()?.let { throw SpeechFailure.Failed(it) }
            val audio = Wav.parse(file.readBytes())
                ?: throw SpeechFailure.Failed("the engine did not produce PCM")
            return SpeechAudio(audio.sampleRate.toUInt(), audio.samples)
        } finally {
            piece = null
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
            try {
                ParcelFileDescriptor.AutoCloseOutputStream(pipe[1]).use { it.write(audio.samples) }
            } catch (_: IOException) {
                // The recognizer stopped reading, which it may do whenever it has its answer.
            }
        }
        val result = AtomicReference<Result<String>?>(null)
        val done = CountDownLatch(1)
        val main = Handler(Looper.getMainLooper())
        var recognizer: SpeechRecognizer? = null
        main.post {
            try {
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
            } catch (e: RuntimeException) {
                // On the main thread this would take the app down.
                pipe[1].close()
                result.set(Result.failure(SpeechFailure.Failed(e.message ?: "the recognizer did not start")))
                done.countDown()
            }
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
                    SpeechRecognizer.ERROR_CLIENT,
                    -> SpeechFailure.Unavailable()
                    SpeechRecognizer.ERROR_RECOGNIZER_BUSY -> SpeechFailure.Busy()
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
