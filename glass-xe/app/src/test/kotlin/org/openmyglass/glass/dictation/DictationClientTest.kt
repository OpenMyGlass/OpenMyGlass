package org.openmyglass.glass.dictation

import com.google.protobuf.ByteString
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.openmyglass.glass.bridge.BridgeServer
import org.openmyglass.protocol.v1.Bridge

class DictationClientTest {
    private val sent = mutableListOf<Bridge.Frame>()
    private val events = mutableListOf<String>()
    private val client = DictationClient(object : DictationClient.Listener {
        override fun onTranscript(requestId: Int, text: String, error: String) { events += "t:$requestId:$text:$error" }
        override fun onResult(requestId: Int, ok: Boolean, message: String) { events += "r:$requestId:$ok:$message" }
        override fun onUnavailable() { events += "unavailable" }
    })
    private val sender = BridgeServer.Sender { sent += it }

    @Test
    fun `a dictation streams start, chunks with sequence numbers and end`() {
        client.onSessionStarted(sender)
        val id = client.start("k1", 2, 16_000)
        assertTrue(id > 0)
        client.audio(id, byteArrayOf(1, 2)); client.audio(id, byteArrayOf(3))
        client.finish(id)
        assertEquals(listOf("k1", "2", "16000"), sent[0].dictationStart.let { listOf(it.key, "${it.actionIndex}", "${it.sampleRate}") })
        assertEquals(listOf(0, 1), sent.filter { it.hasAudioChunk() }.map { it.audioChunk.seq })
        assertFalse(sent.last().dictationEnd.cancelled)
    }

    @Test
    fun `no session means no dictation`() {
        // Review Focus 1: Glass must not record when the phone is not there
        assertEquals(-1, client.start("k1", 0, 16_000))
        assertTrue(sent.isEmpty())
    }

    @Test
    fun `transcripts and results for the current request reach the listener, stale ones do not`() {
        client.onSessionStarted(sender)
        val first = client.start("k1", 0, 16_000)
        val second = client.start("k1", 0, 16_000)
        client.onFrame(transcript(first, "old"))
        client.onFrame(transcript(second, "ciao"))
        client.confirm(second, true)
        client.onFrame(Bridge.Frame.newBuilder().setReplyResult(Bridge.ReplyResult.newBuilder().setRequestId(second).setOk(true).setMessage("Sent")).build())
        assertEquals(listOf("t:$second:ciao:", "r:$second:true:Sent"), events)
        assertTrue(sent.last().replyConfirm.send)
    }

    @Test
    fun `losing the session mid-dictation tells the listener`() {
        client.onSessionStarted(sender)
        client.start("k1", 0, 16_000)
        client.onSessionEnded(sender)
        assertEquals(listOf("unavailable"), events)
        assertFalse(client.audio(1, byteArrayOf(1)))
    }

    @Test
    fun `cancel sends a cancelled end`() {
        client.onSessionStarted(sender)
        val id = client.start("k1", 0, 16_000)
        client.cancel(id)
        assertTrue(sent.last().dictationEnd.cancelled)
    }

    private fun transcript(id: Int, text: String) = Bridge.Frame.newBuilder().setTranscript(
        Bridge.Transcript.newBuilder().setRequestId(id).setText(text).setFinal(true)).build()
}

class SilenceDetectorTest {
    private fun tone(ms: Int, amplitude: Int) = ShortArray(16 * ms) { (if (it % 2 == 0) amplitude else -amplitude).toShort() }

    @Test
    fun `stops after speech followed by two seconds of silence`() {
        val d = SilenceDetector(16_000, 2_000, 30_000)
        assertFalse(d.feed(tone(300, 0)))             // silence before speaking does not stop
        assertFalse(d.feed(tone(800, 3_000)))         // speech
        assertFalse(d.feed(tone(1_500, 50)))          // short pause
        assertTrue(d.feed(tone(600, 50)))             // 2.1 s of silence
    }

    @Test
    fun `stops at the maximum duration`() {
        val d = SilenceDetector(16_000, 2_000, 1_000)
        assertFalse(d.feed(tone(900, 3_000)))
        assertTrue(d.feed(tone(200, 3_000)))
    }

    @Test
    fun `gives up when nothing is said for the whole start window`() {
        val d = SilenceDetector(16_000, 2_000, 30_000, 5_000)
        assertFalse(d.feed(tone(4_000, 20)))
        assertTrue(d.feed(tone(1_100, 20)))
    }
}

class ResamplerTest {
    @Test
    fun `44100 to 16000 keeps the duration`() {
        val out = Resampler.downsample(ShortArray(44_100) { 1000 }, 44_100, 16_000)
        assertEquals(16_000, out.size)
        assertEquals(1000, out[8_000].toInt())
    }

    @Test
    fun `same rate is unchanged`() {
        val input = shortArrayOf(1, 2, 3)
        assertEquals(input.toList(), Resampler.downsample(input, 16_000, 16_000).toList())
    }

    @Test
    fun `pcm16 little endian`() {
        assertEquals(listOf<Byte>(0x34, 0x12, -1, -1), Resampler.toBytes(shortArrayOf(0x1234, -1)).toList())
    }
}
