package org.openmyglass.glass.media

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.openmyglass.glass.bridge.BridgeServer
import org.openmyglass.protocol.v1.Bridge
import java.io.File

class MediaChunkTest {
    @get:Rule val tmp = TemporaryFolder()
    private val sent = mutableListOf<Bridge.Frame>()
    private val status = object : MediaHandler.Status {
        override fun charging() = false
        override fun wifi() = false
        override fun ip() = ""
    }
    private val payload = ByteArray(100_000) { (it % 7).toByte() }
    private lateinit var file: File
    private lateinit var handler: MediaHandler
    private lateinit var id: String

    private fun setUp() {
        val dir = tmp.newFolder("Camera")
        file = File(dir, "v.mp4").apply { writeBytes(payload); setLastModified(1_000_000) }
        val index = MediaIndex(dir)
        id = index.scan().single().id
        handler = MediaHandler(index, status, java.util.concurrent.Executor { it.run() })
        handler.onSessionStarted(BridgeServer.Sender { sent += it })
        sent.clear()
    }

    private fun request(id: String, offset: Long, length: Int) = handler.onFrame(Bridge.Frame.newBuilder().setMediaChunkRequest(
        Bridge.MediaChunkRequest.newBuilder().setId(id).setOffset(offset).setLength(length)).build())

    @Test
    fun `chunks are served from the offset, capped, the hash only in the last one`() {
        setUp()
        request(id, 0, 1_000)
        assertEquals("", sent.single().mediaChunk.sha256)   // hashing waits for the end of the file
        sent.clear()
        request(id, 90_000, 1_000_000)
        val c = sent.single().mediaChunk
        assertEquals(10_000, c.data.size())
        assertEquals(100_000, c.total)
        assertEquals(64, c.sha256.length)
        assertTrue(c.data.toByteArray().contentEquals(payload.copyOfRange(90_000, 100_000)))
    }

    @Test
    fun `requests are bounded to 48 KiB`() {
        setUp()
        request(id, 0, 10_000_000)
        assertEquals(48 * 1024, sent.single().mediaChunk.data.size())
    }

    @Test
    fun `unknown ids and bad offsets get an error chunk`() {
        // Review Focus 3
        setUp()
        request("nope", 0, 1000); request(id, -5, 1000)
        assertTrue(sent.all { it.mediaChunk.error.isNotEmpty() && it.mediaChunk.data.isEmpty })
    }

    @Test
    fun `the end of the file is an empty chunk`() {
        setUp()
        request(id, 100_000, 1000)
        val c = sent.single().mediaChunk
        assertTrue(c.data.isEmpty); assertEquals("", c.error); assertEquals(100_000, c.total); assertEquals(64, c.sha256.length)
    }

    @Test
    fun `delete removes the indexed file version only`() {
        setUp()
        handler.onFrame(Bridge.Frame.newBuilder().setMediaDelete(Bridge.MediaDelete.newBuilder().setId(id)).build())
        assertTrue(sent.single().mediaDeleted.ok)
        assertFalse(file.exists())
    }

    @Test
    fun `a changed file is not deleted`() {
        // Review Focus 2: the phone saved an older version
        setUp()
        file.appendBytes(byteArrayOf(1)); file.setLastModified(2_000_000)
        handler.onFrame(Bridge.Frame.newBuilder().setMediaDelete(Bridge.MediaDelete.newBuilder().setId(id)).build())
        assertFalse(sent.single().mediaDeleted.ok)
        assertTrue(file.exists())
    }

    @Test
    fun `a huge length is an error, not a crash`() {
        // Review finding: uint32 >= 2^31 arrives as a negative int in Java
        setUp()
        request(id, 0, -1)
        assertTrue(sent.single().mediaChunk.error.isNotEmpty())
    }

    @Test
    fun `chunk requests do not block other frames`() {
        // Review finding: hashing a big video on the session thread stalled navigation and notifications
        val dir = tmp.newFolder("Camera2")
        File(dir, "v.mp4").apply { writeBytes(payload); setLastModified(1_000_000) }
        val index = MediaIndex(dir)
        val queued = mutableListOf<Runnable>()
        val h = MediaHandler(index, status, java.util.concurrent.Executor { queued += it })
        h.onSessionStarted(BridgeServer.Sender { sent += it })
        sent.clear()
        h.onFrame(Bridge.Frame.newBuilder().setMediaChunkRequest(Bridge.MediaChunkRequest.newBuilder().setId(index.scan().single().id).setLength(1000)).build())
        h.onFrame(Bridge.Frame.newBuilder().setMediaListRequest(Bridge.MediaListRequest.getDefaultInstance()).build())
        assertTrue(sent.single().hasMediaList())   // answered while the chunk is still queued
        queued.single().run()
        assertTrue(sent.last().hasMediaChunk())
    }
}
