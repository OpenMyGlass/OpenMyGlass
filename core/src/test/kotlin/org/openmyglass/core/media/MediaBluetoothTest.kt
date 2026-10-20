package org.openmyglass.core.media

import com.google.protobuf.ByteString
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.openmyglass.protocol.v1.Bridge
import org.openmyglass.protocol.v1.Bridge.Frame
import java.io.ByteArrayOutputStream
import java.security.MessageDigest
import java.util.Collections

class MediaBluetoothTest {
    private val content = ByteArray(120_000) { (it % 241).toByte() }
    private val sha = MessageDigest.getInstance("SHA-256").digest(content).joinToString("") { "%02x".format(it) }

    /** Partial files in memory; complete() checks the hash like the phone does. */
    private inner class MemoryStore : PartialStore {
        val parts = mutableMapOf<String, ByteArrayOutputStream>()
        val saved = mutableListOf<String>()
        override fun length(id: String) = parts[id]?.size()?.toLong() ?: 0L
        override fun append(id: String, bytes: ByteArray) { parts.getOrPut(id) { ByteArrayOutputStream() }.write(bytes) }
        override fun reset(id: String) { parts.remove(id) }
        override fun complete(item: MediaItem, sha256: String): DownloadResult {
            val data = parts.remove(item.id)?.toByteArray() ?: return DownloadResult.Failed("missing")
            val actual = MessageDigest.getInstance("SHA-256").digest(data).joinToString("") { "%02x".format(it) }
            if (actual != sha256) return DownloadResult.Failed("checksum mismatch")
            saved += item.id
            return DownloadResult.Saved
        }
    }

    private val store = MemoryStore()
    private val synced = Collections.synchronizedSet(mutableSetOf<String>())
    private var deleteAfter = false
    private var dropAfterRequests = Int.MAX_VALUE
    private var corruptFrom = Int.MAX_VALUE
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
    private val sync = MediaSync(
        synced = object : SyncedStore {
            override fun contains(id: String) = id in synced
            override fun add(id: String) { synced += id }
        },
        downloader = { _, _ -> DownloadResult.Failed("wifi not used here") },
        autoEnabled = { false }, phoneOnWifi = { false }, scope = scope,
        partials = store, deleteAfterSync = { deleteAfter }, chunkTimeoutMs = 2_000,
    )
    private val sent = Collections.synchronizedList(mutableListOf<Frame>())
    private var requests = 0
    private var stale: Frame? = null

    @AfterEach
    fun tearDown() = scope.cancel()

    /** A Glass that answers chunk requests from [content]; "dropping" means it stops answering. */
    private fun attach() = runBlocking {
        sync.attach { f ->
            sent += f
            if (f.hasMediaChunkRequest()) {
                requests++
                if (requests > dropAfterRequests) return@attach
                val r = f.mediaChunkRequest
                val from = r.offset.toInt(); val to = minOf(content.size, from + r.length)
                val data = content.copyOfRange(from, to).also { if (requests >= corruptFrom) it[0] = (it[0] + 1).toByte() }
                stale?.let { sync.onFrame(it); stale = null }   // a late answer to an earlier request arrives first
                sync.onFrame(Frame.newBuilder().setMediaChunk(Bridge.MediaChunk.newBuilder().setId(r.id).setOffset(r.offset)
                    .setData(ByteString.copyFrom(data)).setTotal(content.size.toLong())
                    .setSha256(if (to == content.size) sha else "")).build())
            }
        }
    }
    private fun list() = runBlocking { sync.onFrame(Frame.newBuilder().setMediaList(Bridge.MediaList.newBuilder()
        .addItems(Bridge.MediaItem.newBuilder().setId("v1").setName("v1.mp4").setVideo(true).setSize(content.size.toLong()).setTakenAtMs(1))).build()) }

    @Test
    fun `sync now over bluetooth pulls in chunks and saves`() {
        attach()
        runBlocking { sync.syncNow(overBluetooth = true) }
        list()
        assertEquals(listOf("v1"), store.saved)
        assertTrue(synced.contains("v1"))
        assertTrue(sent.filter { it.hasMediaChunkRequest() }.all { it.mediaChunkRequest.length <= MediaSync.CHUNK_BYTES })
        assertTrue(sent.none { it.hasMediaPullStart() })   // no Wi-Fi server involved
    }

    @Test
    fun `an interrupted transfer resumes from the partial length`() {
        // Review Focus 1
        dropAfterRequests = 1
        attach()
        runBlocking { sync.syncNow(overBluetooth = true) }
        list()
        assertEquals(MediaSync.CHUNK_BYTES.toLong(), store.length("v1"))
        assertTrue(synced.isEmpty())
        Thread.sleep(2_500)                                  // the run gives up after the chunk timeout
        dropAfterRequests = Int.MAX_VALUE; requests = 0; sent.clear()
        runBlocking { sync.syncNow(overBluetooth = true) }
        list()
        assertEquals(MediaSync.CHUNK_BYTES.toLong(), sent.first { it.hasMediaChunkRequest() }.mediaChunkRequest.offset)
        assertEquals(listOf("v1"), store.saved)
    }

    @Test
    fun `a corrupted transfer is not saved`() {
        // Review Focus 2
        corruptFrom = 2
        attach()
        runBlocking { sync.syncNow(overBluetooth = true) }
        list()
        assertTrue(store.saved.isEmpty()); assertTrue(synced.isEmpty())
    }

    @Test
    fun `delete after sync sends a delete only when enabled and only after saving`() {
        // Review Focus 4
        attach()
        runBlocking { sync.syncNow(overBluetooth = true) }; list()
        assertTrue(sent.none { it.hasMediaDelete() })
        synced.clear(); store.saved.clear(); sent.clear(); requests = 0
        deleteAfter = true
        runBlocking { sync.syncNow(overBluetooth = true) }; list()
        assertEquals("v1", sent.last().mediaDelete.id)
        assertTrue(sent.indexOfLast { it.hasMediaChunkRequest() } < sent.indexOfFirst { it.hasMediaDelete() })
    }

    @Test
    fun `wifi runs also delete after sync when enabled`() {
        deleteAfter = true
        val wifiSync = MediaSync(
            synced = object : SyncedStore {
                override fun contains(id: String) = false
                override fun add(id: String) {}
            },
            downloader = { _, _ -> DownloadResult.Saved }, autoEnabled = { true }, phoneOnWifi = { true }, scope = scope,
            partials = store, deleteAfterSync = { deleteAfter })
        val frames = mutableListOf<Frame>()
        runBlocking {
            wifiSync.attach { frames += it }
            wifiSync.syncNow()
            wifiSync.onFrame(Frame.newBuilder().setMediaList(Bridge.MediaList.newBuilder().addItems(Bridge.MediaItem.newBuilder().setId("p1").setName("p.jpg"))).build())
            wifiSync.onFrame(Frame.newBuilder().setMediaServer(Bridge.MediaServer.newBuilder().setIp("1.2.3.4").setPort(1).setToken("t")).build())
        }
        assertEquals("p1", frames.first { it.hasMediaDelete() }.mediaDelete.id)
    }

    @Test
    fun `a late chunk from an earlier request is ignored`() {
        // Review finding: one late answer made every following item fail
        attach()
        stale = Frame.newBuilder().setMediaChunk(Bridge.MediaChunk.newBuilder().setId("old").setOffset(4096)
            .setData(ByteString.copyFrom(ByteArray(10))).setTotal(99)).build()
        runBlocking { sync.syncNow(overBluetooth = true) }
        list()
        assertEquals(listOf("v1"), store.saved)
    }

    @Test
    fun `a partial longer than the file is reset`() {
        // Review finding: the "bad offset" error was checked before the reset, so the item failed forever
        store.append("v1", ByteArray(content.size + 10))
        val sync2 = MediaSync(
            synced = object : SyncedStore {
                override fun contains(id: String) = false
                override fun add(id: String) {}
            },
            downloader = { _, _ -> DownloadResult.Failed("") }, autoEnabled = { false }, phoneOnWifi = { false }, scope = scope,
            partials = store, chunkTimeoutMs = 2_000)
        runBlocking {
            sync2.attach { f ->
                if (f.hasMediaChunkRequest()) sync2.onFrame(Frame.newBuilder().setMediaChunk(Bridge.MediaChunk.newBuilder()
                    .setId("v1").setOffset(f.mediaChunkRequest.offset).setTotal(content.size.toLong()).setError("bad offset")).build())
            }
            sync2.syncNow(overBluetooth = true)
            sync2.onFrame(Frame.newBuilder().setMediaList(Bridge.MediaList.newBuilder().addItems(Bridge.MediaItem.newBuilder().setId("v1").setName("v1.mp4"))).build())
        }
        assertEquals(0, store.length("v1"))
    }

    @Test
    fun `losing the session mid-chunk fails the item at once`() {
        // With a 120 s chunk timeout, a detach must not leave the run waiting that long
        val slow = MediaSync(
            synced = object : SyncedStore {
                override fun contains(id: String) = false
                override fun add(id: String) {}
            },
            downloader = { _, _ -> DownloadResult.Failed("") }, autoEnabled = { false }, phoneOnWifi = { false }, scope = scope,
            partials = store, chunkTimeoutMs = 60_000)
        runBlocking {
            slow.attach { }                                  // Glass never answers chunk requests
            slow.syncNow(overBluetooth = true)
            slow.onFrame(Frame.newBuilder().setMediaList(Bridge.MediaList.newBuilder().addItems(Bridge.MediaItem.newBuilder().setId("v1").setName("v1.mp4"))).build())
        }
        val started = System.nanoTime()
        runBlocking { slow.detach() }
        Thread.sleep(300)
        assertTrue(slow.status.value.contains("failed"), slow.status.value)
        assertTrue((System.nanoTime() - started) / 1_000_000 < 5_000)
    }
}
