package org.openmyglass.core.dictation

import com.google.protobuf.ByteString
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.openmyglass.protocol.v1.Bridge
import org.openmyglass.protocol.v1.Bridge.Frame
import java.util.Collections
import kotlin.time.Duration.Companion.milliseconds

class DictationServiceTest {
    /** Fake engine: the "speech" is the PCM bytes read as text. */
    private class FakeEngine : SpeechToText {
        val cancelled = mutableListOf<Int>()
        var rate = 0
        override fun start(sampleRate: Int): Recognition {
            rate = sampleRate
            val id = cancelled.size
            return object : Recognition {
                val sb = StringBuilder()
                override fun feed(pcm: ByteArray) { sb.append(String(pcm)) }
                override fun finish() = sb.toString().trim()
                override fun cancel() { cancelled += id }
            }
        }
    }

    private val engine = FakeEngine()
    private var available: SpeechToText? = engine
    private val replies = Collections.synchronizedList(mutableListOf<Triple<String, Int, String>>())
    private var replyOutcome = ReplyOutcome(true, "Sent")
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val service = DictationService(
        engine = { available ?: throw SpeechUnavailable("Download the Italian speech model in OpenMyGlass settings") },
        replies = { key, index, text -> replies += Triple(key, index, text); replyOutcome },
        scope = scope, timeout = 300.milliseconds,
    )
    private val sent = Collections.synchronizedList(mutableListOf<Frame>())
    private val send: suspend (Frame) -> Unit = { sent += it }

    @AfterEach
    fun tearDown() = scope.cancel()

    private fun start(id: Int, key: String = "k1", index: Int = 0, free: Boolean = false) = Frame.newBuilder().setDictationStart(
        Bridge.DictationStart.newBuilder().setRequestId(id).setKey(key).setActionIndex(index).setSampleRate(16_000).setFree(free)).build()
    private fun chunk(id: Int, seq: Int, text: String) = Frame.newBuilder().setAudioChunk(
        Bridge.AudioChunk.newBuilder().setRequestId(id).setSeq(seq).setPcm(ByteString.copyFromUtf8(text))).build()
    private fun end(id: Int, cancelled: Boolean = false) = Frame.newBuilder().setDictationEnd(
        Bridge.DictationEnd.newBuilder().setRequestId(id).setCancelled(cancelled)).build()
    private fun confirm(id: Int, send: Boolean) = Frame.newBuilder().setReplyConfirm(
        Bridge.ReplyConfirm.newBuilder().setRequestId(id).setSend(send)).build()

    private fun feed(vararg frames: Frame) = runBlocking { frames.forEach { service.onFrame(it, send) } }
    private fun transcripts() = sent.filter { it.hasTranscript() }.map { it.transcript }
    private fun results() = sent.filter { it.hasReplyResult() }.map { it.replyResult }

    @Test
    fun `audio is transcribed and confirmed replies are sent`() {
        feed(start(1, "k1", 2), chunk(1, 0, "arrivo "), chunk(1, 1, "tra dieci minuti"), end(1))
        assertEquals(16_000, engine.rate)
        val t = transcripts().single()
        assertEquals("arrivo tra dieci minuti", t.text); assertTrue(t.final); assertEquals("", t.error)
        feed(confirm(1, true))
        assertEquals(listOf(Triple("k1", 2, "arrivo tra dieci minuti")), replies.toList())
        assertTrue(results().single().ok)
    }

    @Test
    fun `cancel at confirm sends nothing`() {
        feed(start(1), chunk(1, 0, "ciao"), end(1), confirm(1, false))
        assertTrue(replies.isEmpty()); assertTrue(results().isEmpty())
    }

    @Test
    fun `no speech model answers at once with an error`() {
        // Review Focus 3: Glass must not record for nothing
        available = null
        feed(start(1))
        val t = transcripts().single()
        assertTrue(t.error.contains("speech model"), t.error)
        assertTrue(t.final)
    }

    @Test
    fun `silence gives an empty transcript with an error and cannot be sent`() {
        // Review Focus 4
        feed(start(1), chunk(1, 0, "   "), end(1))
        assertEquals("Nothing heard", transcripts().single().error)
        feed(confirm(1, true))
        assertTrue(replies.isEmpty())
        assertFalse(results().single().ok)
    }

    @Test
    fun `a newer dictation cancels the older one`() {
        // Review Focus 5
        feed(start(1), chunk(1, 0, "uno"), start(2), chunk(1, 1, "late"), chunk(2, 0, "due"), end(1), end(2))
        assertEquals(listOf(0), engine.cancelled)
        assertEquals(listOf(2), transcripts().map { it.requestId })
        assertEquals("due", transcripts().single().text)
    }

    @Test
    fun `glass cancelling the dictation discards it`() {
        feed(start(1), chunk(1, 0, "ciao"), end(1, cancelled = true), confirm(1, true))
        assertTrue(transcripts().isEmpty()); assertTrue(replies.isEmpty())
        assertFalse(results().single().ok)
    }

    @Test
    fun `an abandoned dictation expires`() {
        // Review Focus 1: the link dropped mid-dictation; nothing may be sent later
        feed(start(1), chunk(1, 0, "ciao"))
        Thread.sleep(500)
        feed(end(1), confirm(1, true))
        assertTrue(replies.isEmpty())
        assertEquals(listOf(0), engine.cancelled)
    }

    @Test
    fun `session end cancels the active dictation`() {
        feed(start(1), chunk(1, 0, "ciao"))
        runBlocking { service.sessionEnded() }
        feed(end(1), confirm(1, true))
        assertTrue(replies.isEmpty())
        assertEquals(listOf(0), engine.cancelled)
    }

    @Test
    fun `a failed reply is reported to glass`() {
        // Review Focus 2: the notification was removed between Reply and Send
        replyOutcome = ReplyOutcome(false, "The notification is gone")
        feed(start(1), chunk(1, 0, "ok"), end(1), confirm(1, true))
        assertEquals("The notification is gone", results().single().message)
        assertFalse(results().single().ok)
    }

    @Test
    fun `an engine failure answers with an error instead of breaking the link`() {
        // Review finding: a native Error from the recognizer escaped onFrame and dropped the bridge session
        val broken = DictationService(
            engine = { SpeechToText { object : Recognition {
                override fun feed(pcm: ByteArray) = throw UnsatisfiedLinkError("libvosk")
                override fun finish() = throw IllegalStateException("secret transcript text")
                override fun cancel() {}
            } } },
            replies = { _, _, _ -> ReplyOutcome(true, "") }, scope = scope)
        runBlocking {
            broken.onFrame(start(1), send)
            broken.onFrame(chunk(1, 0, "x"), send)   // must not throw
        }
        val t = transcripts().single()
        assertEquals("Speech recognition failed", t.error)
        assertFalse(t.error.contains("secret"))
    }

    @Test
    fun `the expiry restarts when the transcript is ready`() {
        // Issue #59: a long dictation left little time to review
        feed(start(1), chunk(1, 0, "ciao"))
        Thread.sleep(200)                       // most of the 300 ms timeout spent dictating
        feed(end(1))
        Thread.sleep(200)                       // reviewing: 400 ms since start, 200 ms since the transcript
        feed(confirm(1, true))
        assertEquals(listOf(Triple("k1", 0, "ciao")), replies.toList())
    }

    @Test
    fun `free dictation ends with the transcript and never replies`() {
        feed(start(1, key = "", free = true), chunk(1, 0, "comprare il latte"), end(1))
        val t = transcripts().single()
        assertEquals("comprare il latte", t.text); assertTrue(t.final); assertEquals("", t.error)
        // The request is already forgotten: a stray confirm cannot turn the note into a reply
        feed(confirm(1, true))
        assertTrue(replies.isEmpty())
        assertFalse(results().single().ok)
        assertTrue(engine.cancelled.isEmpty())
    }
}
