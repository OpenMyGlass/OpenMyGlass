package org.openmyglass.core.dictation

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.openmyglass.protocol.v1.Bridge
import org.openmyglass.protocol.v1.Bridge.Frame
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds

/**
 * Phone side of a dictated reply: Glass streams audio, the phone transcribes it offline with [engine], Glass
 * confirms, and [replies] fires the notification's RemoteInput. One request at a time (a newer one cancels the
 * older); a request is forgotten after [timeout]. Audio and text are never logged or stored.
 */
class DictationService(
    private val engine: () -> SpeechToText,
    private val replies: suspend (key: String, actionIndex: Int, text: String) -> ReplyOutcome,
    private val scope: CoroutineScope,
    private val timeout: Duration = 60.seconds,
) {
    private class Request(val id: Int, val key: String, val actionIndex: Int, val recognition: Recognition?) {
        var text: String? = null      // set when transcribed; "" means nothing usable
        var ended = false
        var expiry: Job? = null
    }

    private val lock = Mutex()
    private var current: Request? = null

    suspend fun onFrame(frame: Frame, send: suspend (Frame) -> Unit) {
        try {
            handle(frame, send)
        } catch (e: kotlinx.coroutines.CancellationException) {
            throw e
        } catch (_: Throwable) {
            // A recognizer failure (native errors included) must not break the bridge session, and its message
            // may contain transcript text: answer with a fixed error and drop the request.
            val id = lock.withLock { current?.id.also { drop() } } ?: return
            send(transcript(id, "", "Speech recognition failed"))
        }
    }

    private suspend fun handle(frame: Frame, send: suspend (Frame) -> Unit) {
        when {
            frame.hasDictationStart() -> start(frame.dictationStart, send)
            frame.hasAudioChunk() -> lock.withLock {
                current?.takeIf { it.id == frame.audioChunk.requestId && !it.ended }?.recognition?.feed(frame.audioChunk.pcm.toByteArray())
            }
            frame.hasDictationEnd() -> end(frame.dictationEnd, send)
            frame.hasReplyConfirm() -> confirm(frame.replyConfirm, send)
        }
    }

    /** The bridge session ended: whatever was being dictated is dropped. */
    suspend fun sessionEnded() = lock.withLock { drop() }

    private suspend fun start(s: Bridge.DictationStart, send: suspend (Frame) -> Unit) {
        val failure = lock.withLock {
            drop()
            val recognition = try {
                engine().start(s.sampleRate)
            } catch (e: SpeechUnavailable) {
                return@withLock e.message ?: "Speech recognition unavailable"
            }
            current = Request(s.requestId, s.key, s.actionIndex, recognition).also { r ->
                r.expiry = scope.launch {
                    delay(timeout)
                    lock.withLock { if (current === r) drop() }
                }
            }
            null
        }
        if (failure != null) send(transcript(s.requestId, "", failure))
    }

    private suspend fun end(e: Bridge.DictationEnd, send: suspend (Frame) -> Unit) {
        val text = lock.withLock {
            val r = current?.takeIf { it.id == e.requestId && !it.ended } ?: return
            if (e.cancelled) {
                drop()
                return
            }
            r.ended = true
            r.recognition?.finish()?.trim().orEmpty().also { r.text = it }
        }
        send(transcript(e.requestId, text, if (text.isEmpty()) "Nothing heard" else ""))
    }

    private suspend fun confirm(c: Bridge.ReplyConfirm, send: suspend (Frame) -> Unit) {
        val r = lock.withLock {
            val r = current?.takeIf { it.id == c.requestId }
            if (r != null) {
                r.expiry?.cancel()
                current = null
            }
            r
        }
        if (!c.send) return
        val outcome = when {
            r == null -> ReplyOutcome(false, "This reply expired, dictate again")
            r.text.isNullOrEmpty() -> ReplyOutcome(false, "Nothing to send")
            else -> runCatching { replies(r.key, r.actionIndex, r.text!!) }.getOrElse { ReplyOutcome(false, "Reply failed") }
        }
        send(Frame.newBuilder().setReplyResult(
            Bridge.ReplyResult.newBuilder().setRequestId(c.requestId).setOk(outcome.ok).setMessage(outcome.message)).build())
    }

    /** Caller holds the lock. */
    private fun drop() {
        current?.let { r ->
            r.expiry?.cancel()
            if (!r.ended) runCatching { r.recognition?.cancel() }
        }
        current = null
    }

    private fun transcript(id: Int, text: String, error: String) = Frame.newBuilder().setTranscript(
        Bridge.Transcript.newBuilder().setRequestId(id).setText(text).setFinal(true).setError(error)).build()
}
