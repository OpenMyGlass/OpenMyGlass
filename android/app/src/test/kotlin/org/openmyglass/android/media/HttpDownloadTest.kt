package org.openmyglass.android.media

import com.sun.net.httpserver.HttpServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.net.InetSocketAddress
import java.security.MessageDigest

class HttpDownloadTest {
    @get:Rule val tmp = TemporaryFolder()
    private val payload = ByteArray(200_000) { (it % 253).toByte() }
    private val sha = MessageDigest.getInstance("SHA-256").digest(payload).joinToString("") { "%02x".format(it) }
    private val ranges = mutableListOf<String?>()
    private var advertisedSha = sha
    private var cutAfter = Int.MAX_VALUE
    private val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0).apply {
        createContext("/media/x") { ex ->
            val range = ex.requestHeaders.getFirst("Range"); ranges += range
            val from = range?.removePrefix("bytes=")?.substringBefore('-')?.toInt() ?: 0
            ex.responseHeaders.add("X-Content-SHA256", advertisedSha)
            ex.sendResponseHeaders(if (from > 0) 206 else 200, (payload.size - from).toLong())
            val body = payload.copyOfRange(from, payload.size)
            ex.responseBody.use { out ->
                if (body.size > cutAfter) { out.write(body, 0, cutAfter); out.flush(); ex.close(); return@createContext }
                out.write(body)
            }
        }
        start()
    }
    private val url get() = "http://127.0.0.1:${server.address.port}/media/x?token=t"

    @After
    fun tearDown() = server.stop(0)

    @Test
    fun `downloads and verifies the hash`() {
        val partial = tmp.root.resolve("x.part")
        val r = HttpDownload.fetch(url, partial)
        assertTrue(r is HttpDownload.Result.Ok)
        assertTrue(partial.readBytes().contentEquals(payload))
    }

    @Test
    fun `an interrupted download resumes with a range request`() {
        // Review Focus 1
        val partial = tmp.root.resolve("x.part")
        cutAfter = 50_000
        assertTrue(HttpDownload.fetch(url, partial) is HttpDownload.Result.Failed)
        assertTrue(partial.length() in 1..50_000)
        cutAfter = Int.MAX_VALUE
        val r = HttpDownload.fetch(url, partial)
        assertTrue(r is HttpDownload.Result.Ok)
        assertTrue(ranges.last()!!.startsWith("bytes="))
        assertTrue(partial.readBytes().contentEquals(payload))
    }

    @Test
    fun `a hash mismatch fails and discards the partial file`() {
        // Review Focus 2: retried from scratch next time, never saved
        advertisedSha = "00".repeat(32)
        val partial = tmp.root.resolve("x.part")
        val r = HttpDownload.fetch(url, partial)
        assertTrue(r is HttpDownload.Result.Failed)
        assertFalse(partial.exists())
    }
}
