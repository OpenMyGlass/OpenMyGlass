package org.openmyglass.glass.media

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.openmyglass.glass.bridge.BridgeServer
import org.openmyglass.protocol.v1.Bridge
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

class MediaIndexTest {
    @get:Rule val tmp = TemporaryFolder()

    @Test
    fun `scan lists photos and videos oldest first with stable ids`() {
        val dir = tmp.newFolder("Camera")
        File(dir, "b.mp4").apply { writeBytes(ByteArray(20)); setLastModified(2_000_000) }
        File(dir, "a.jpg").apply { writeBytes(ByteArray(10)); setLastModified(1_000_000) }
        File(dir, "notes.txt").writeText("x")
        val index = MediaIndex(dir)
        val items = index.scan()
        assertEquals(listOf("a.jpg", "b.mp4"), items.map { it.name })
        assertEquals(listOf(false, true), items.map { it.video })
        assertEquals(items.map { it.id }, MediaIndex(dir).scan().map { it.id })
        assertEquals(File(dir, "a.jpg"), index.file(items[0].id))
    }

    @Test
    fun `a changed file gets a new id and unknown ids resolve to nothing`() {
        val dir = tmp.newFolder("Camera")
        val f = File(dir, "a.jpg").apply { writeBytes(ByteArray(10)) }
        val index = MediaIndex(dir)
        val before = index.scan().single().id
        f.writeBytes(ByteArray(11))
        assertNotEquals(before, index.scan().single().id)
        assertEquals(null, index.file("../../etc/passwd"))
    }
}

class MediaHttpServerTest {
    @get:Rule val tmp = TemporaryFolder()
    private lateinit var server: MediaHttpServer
    private val payload = ByteArray(100_000) { (it % 251).toByte() }

    private fun setUp(): Pair<String, Int> {
        val dir = tmp.newFolder("Camera")
        File(dir, "a.jpg").writeBytes(payload)
        val index = MediaIndex(dir)
        val id = index.scan().single().id
        server = MediaHttpServer(index, "secret-token", 120_000)
        return id to server.start()
    }

    @After
    fun tearDown() { if (::server.isInitialized) server.close() }

    private fun get(port: Int, path: String, range: String? = null): HttpURLConnection =
        (URL("http://127.0.0.1:$port$path").openConnection() as HttpURLConnection).apply { range?.let { setRequestProperty("Range", it) } }

    @Test
    fun `serves the file with its sha256`() {
        val (id, port) = setUp()
        val c = get(port, "/media/$id?token=secret-token")
        assertEquals(200, c.responseCode)
        assertTrue(c.inputStream.readBytes().contentEquals(payload))
        val sha = MessageDigest.getInstance("SHA-256").digest(payload).joinToString("") { "%02x".format(it) }
        assertEquals(sha, c.getHeaderField("X-Content-SHA256"))
    }

    @Test
    fun `range requests resume`() {
        // Review Focus 1
        val (id, port) = setUp()
        val c = get(port, "/media/$id?token=secret-token", "bytes=60000-")
        assertEquals(206, c.responseCode)
        assertTrue(c.inputStream.readBytes().contentEquals(payload.copyOfRange(60_000, payload.size)))
        assertEquals("bytes 60000-99999/100000", c.getHeaderField("Content-Range"))
    }

    @Test
    fun `wrong token, unknown id and path tricks are refused`() {
        // Review Focus 4
        val (id, port) = setUp()
        assertEquals(403, get(port, "/media/$id?token=nope").responseCode)
        assertEquals(403, get(port, "/media/$id").responseCode)
        assertEquals(404, get(port, "/media/unknown?token=secret-token").responseCode)
        assertEquals(404, get(port, "/media/..%2F..%2Fetc%2Fpasswd?token=secret-token").responseCode)
        assertEquals(404, get(port, "/other?token=secret-token").responseCode)
    }

    @Test
    fun `closes itself when idle`() {
        val dir = tmp.newFolder("Camera")
        server = MediaHttpServer(MediaIndex(dir), "t", 200)
        val port = server.start()
        Thread.sleep(600)
        val refused = try { get(port, "/media/x?token=t").responseCode; false } catch (_: java.io.IOException) { true }
        assertTrue(refused)
    }
}

class MediaHandlerTest {
    @get:Rule val tmp = TemporaryFolder()
    private val sent = mutableListOf<Bridge.Frame>()
    private val sender = BridgeServer.Sender { sent += it }
    private var wifi = true
    private val status = object : MediaHandler.Status {
        override fun charging() = true
        override fun wifi() = wifi
        override fun ip() = if (wifi) "127.0.0.1" else ""
    }

    private fun handler(): MediaHandler {
        val dir = tmp.newFolder("Camera")
        File(dir, "a.jpg").writeBytes(ByteArray(10))
        return MediaHandler(MediaIndex(dir), status)
    }

    private fun frame(b: Bridge.Frame.Builder.() -> Unit) = Bridge.Frame.newBuilder().apply(b).build()

    @Test
    fun `session start reports status, a request gets the list`() {
        val h = handler()
        h.onSessionStarted(sender)
        assertTrue(sent.single().glassStatus.charging)
        h.onFrame(frame { mediaListRequest = Bridge.MediaListRequest.getDefaultInstance() })
        assertEquals("a.jpg", sent.last().mediaList.itemsList.single().name)
    }

    @Test
    fun `a pull opens a server with a fresh token and done closes it`() {
        val h = handler()
        h.onSessionStarted(sender)
        h.onFrame(frame { mediaPullStart = Bridge.MediaPullStart.getDefaultInstance() })
        val s = sent.last().mediaServer
        assertEquals("127.0.0.1", s.ip)
        assertEquals(32, s.token.length)
        h.onFrame(frame { mediaPullStart = Bridge.MediaPullStart.getDefaultInstance() })
        assertNotEquals(s.token, sent.last().mediaServer.token)
        h.onFrame(frame { mediaPullDone = Bridge.MediaPullDone.getDefaultInstance() })
        val refused = try { (URL("http://127.0.0.1:${sent.last().mediaServer.port}/").openConnection() as HttpURLConnection).responseCode; false } catch (_: java.io.IOException) { true }
        assertTrue(refused)
    }

    @Test
    fun `no wifi means no server`() {
        wifi = false
        val h = handler()
        h.onSessionStarted(sender)
        h.onFrame(frame { mediaPullStart = Bridge.MediaPullStart.getDefaultInstance() })
        assertEquals("Glass is not on Wi-Fi", sent.last().mediaServerUnavailable.reason)
    }

    @Test
    fun `new media and status changes are announced only with a session`() {
        val h = handler()
        h.onNewMedia(); h.onStatusChanged()
        assertTrue(sent.isEmpty())
        h.onSessionStarted(sender)
        h.onNewMedia()
        assertTrue(sent.last().hasMediaList())
        h.onSessionEnded(sender)
        h.onStatusChanged()
        assertFalse(sent.last().hasGlassStatus() && sent.size > 2)
    }
}
