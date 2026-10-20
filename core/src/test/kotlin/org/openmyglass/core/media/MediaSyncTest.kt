package org.openmyglass.core.media

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

class MediaSyncTest {
    private val synced = Collections.synchronizedSet(mutableSetOf<String>())
    private val downloaded = Collections.synchronizedList(mutableListOf<String>())
    private var failIds = setOf<String>()
    private var autoEnabled = true
    private var phoneWifi = true
    private var onDownload: (String) -> Unit = {}
    private var gate: kotlinx.coroutines.CompletableDeferred<Unit>? = null
    private val crashes = Collections.synchronizedList(mutableListOf<Throwable>())
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined +
        kotlinx.coroutines.CoroutineExceptionHandler { _, e -> crashes += e })
    private val sync = MediaSync(
        synced = object : SyncedStore {
            override fun contains(id: String) = id in synced
            override fun add(id: String) { synced += id }
        },
        downloader = { server, item ->
            gate?.await()
            onDownload(item.id)
            if (item.id in failIds) DownloadResult.Failed("hash mismatch")
            else { downloaded += "${server.token}:${item.id}"; DownloadResult.Saved }
        },
        autoEnabled = { autoEnabled }, phoneOnWifi = { phoneWifi }, scope = scope,
    )
    private val sent = Collections.synchronizedList(mutableListOf<Frame>())

    @AfterEach
    fun tearDown() = scope.cancel()

    private fun attach() = runBlocking { sync.attach { sent += it } }
    private fun glass(f: Frame) = runBlocking { sync.onFrame(f) }
    private fun status(charging: Boolean, wifi: Boolean = true) = Frame.newBuilder().setGlassStatus(
        Bridge.GlassStatus.newBuilder().setCharging(charging).setWifi(wifi).setIp("192.168.1.20")).build()
    private fun list(vararg ids: Pair<String, Long>) = Frame.newBuilder().setMediaList(Bridge.MediaList.newBuilder().addAllItems(
        ids.map { (id, t) -> Bridge.MediaItem.newBuilder().setId(id).setName("$id.jpg").setSize(10).setTakenAtMs(t).build() })).build()
    private val server = Frame.newBuilder().setMediaServer(Bridge.MediaServer.newBuilder().setIp("192.168.1.20").setPort(8080).setToken("tok")).build()
    private fun sentKinds() = sent.map { it.bodyCase.name }

    @Test
    fun `charging on wifi triggers a sync of unsynced media, oldest first`() {
        synced += "b"
        attach()
        glass(status(charging = true))
        assertEquals(listOf("MEDIA_LIST_REQUEST"), sentKinds())
        glass(list("c" to 30, "a" to 10, "b" to 20))
        assertEquals("MEDIA_PULL_START", sentKinds().last())
        glass(server)
        assertEquals(listOf("tok:a", "tok:c"), downloaded)
        assertEquals(setOf("a", "b", "c"), synced.toSet())
        assertEquals("MEDIA_PULL_DONE", sentKinds().last())
    }

    @Test
    fun `no auto sync when not charging, glass off wifi, phone off wifi or disabled`() {
        attach()
        glass(status(charging = false)); glass(status(charging = true, wifi = false))
        phoneWifi = false; glass(status(charging = true))
        phoneWifi = true; autoEnabled = false; glass(status(charging = true))
        assertTrue(sent.isEmpty())
    }

    @Test
    fun `nothing to sync means no server is opened`() {
        synced += "a"
        attach(); glass(status(charging = true)); glass(list("a" to 1))
        assertEquals(listOf("MEDIA_LIST_REQUEST"), sentKinds())
    }

    @Test
    fun `a failed item is not marked synced and is retried next time`() {
        // Review Focus 2
        failIds = setOf("a")
        attach(); glass(status(charging = true)); glass(list("a" to 1, "b" to 2)); glass(server)
        assertEquals(setOf("b"), synced.toSet())
        failIds = emptySet(); sent.clear()
        glass(status(charging = false)); glass(status(charging = true)); glass(list("a" to 1, "b" to 2)); glass(server)
        assertEquals(setOf("a", "b"), synced.toSet())
    }

    @Test
    fun `charging stopping mid-sync finishes the current item then stops`() {
        // Review Focus 5
        attach(); glass(status(charging = true)); glass(list("a" to 1, "b" to 2, "c" to 3))
        onDownload = { id -> if (id == "a") runBlocking { sync.onFrame(status(charging = false)) } }
        glass(server)
        assertEquals(setOf("a"), synced.toSet())
        assertEquals("MEDIA_PULL_DONE", sentKinds().last())
    }

    @Test
    fun `sync now works without charging and ignores the auto setting`() {
        autoEnabled = false
        attach(); glass(status(charging = false))
        runBlocking { sync.syncNow() }
        glass(list("a" to 1)); glass(server)
        assertEquals(setOf("a"), synced.toSet())
    }

    @Test
    fun `new media announced while charging are synced`() {
        // Review Focus 3: an unsolicited list after a new photo
        attach(); glass(status(charging = true)); glass(list()); sent.clear()
        glass(list("new" to 5)); glass(server)
        assertEquals(setOf("new"), synced.toSet())
    }

    @Test
    fun `server unavailable ends the run`() {
        attach(); runBlocking { sync.syncNow() }; glass(list("a" to 1))
        glass(Frame.newBuilder().setMediaServerUnavailable(Bridge.MediaServerUnavailable.newBuilder().setReason("Glass is not on Wi-Fi")).build())
        assertTrue(synced.isEmpty())
        assertEquals("Glass is not on Wi-Fi", sync.status.value)
        assertFalse(sentKinds().contains("MEDIA_PULL_DONE"))
    }

    @Test
    fun `a broken link at the end of a run does not crash`() {
        // Review finding: an exception from send() in the download coroutine was uncaught
        runBlocking { sync.attach { f -> if (f.hasMediaPullDone()) throw java.io.IOException("link gone") else sent += f } }
        glass(status(charging = true)); glass(list("a" to 1)); glass(server)
        assertTrue(crashes.isEmpty(), crashes.toString())
        assertEquals(setOf("a"), synced.toSet())
    }

    @Test
    fun `a throwing downloader fails the item, not the run`() {
        onDownload = { id -> if (id == "a") throw IllegalStateException("gallery insert failed") }
        attach(); glass(status(charging = true)); glass(list("a" to 1, "b" to 2)); glass(server)
        assertTrue(crashes.isEmpty(), crashes.toString())
        assertEquals(setOf("b"), synced.toSet())
    }

    @Test
    fun `phone joining wifi or the setting turning on starts a run`() {
        // Review finding: only Glass events used to trigger a run
        phoneWifi = false
        attach(); glass(status(charging = true))
        assertTrue(sent.isEmpty())
        phoneWifi = true
        runBlocking { sync.conditionsChanged() }
        assertEquals(listOf("MEDIA_LIST_REQUEST"), sentKinds())
    }

    @Test
    fun `a stale run after a reconnect does not touch the new run`() {
        // Review finding: the old download coroutine sent PullDone over the new link and reset the phase
        gate = kotlinx.coroutines.CompletableDeferred()
        attach(); glass(status(charging = true)); glass(list("a" to 1)); glass(server)   // run 1 blocked in download
        val newLink = Collections.synchronizedList(mutableListOf<Frame>())
        runBlocking { sync.attach { newLink += it } }
        glass(status(charging = true))                                                     // run 2 starts listing
        gate!!.complete(Unit)                                                             // run 1 finishes late
        assertTrue(newLink.none { it.hasMediaPullDone() }, newLink.map { it.bodyCase }.toString())
        glass(list("a" to 1, "b" to 2))                                                   // run 2 continues normally ("a" was saved by run 1)
        assertEquals("MEDIA_PULL_START", newLink.last().bodyCase.name)
    }

    private fun quickSync() = MediaSync(
        synced = object : SyncedStore {
            override fun contains(id: String) = id in synced
            override fun add(id: String) { synced += id }
        },
        downloader = { _, item -> if (item.id in failIds) DownloadResult.Failed("x") else DownloadResult.Saved },
        autoEnabled = { true }, phoneOnWifi = { true }, scope = scope,
        replyTimeoutMs = 200, followUpDelayMs = 200)

    @Test
    fun `a run that Glass never answers times out`() {
        // Issue #64
        val q = quickSync()
        runBlocking { q.attach { sent += it }; q.syncNow() }
        Thread.sleep(400)
        assertEquals("Glass did not answer", q.status.value)
        sent.clear()
        runBlocking { q.syncNow() }
        assertEquals(listOf("MEDIA_LIST_REQUEST"), sentKinds())
    }

    @Test
    fun `failed items get one follow-up run while conditions hold`() {
        // Issue #66
        failIds = setOf("a")
        val q = quickSync()
        runBlocking {
            q.attach { sent += it }
            q.onFrame(status(charging = true)); q.onFrame(list("a" to 1)); q.onFrame(server)
        }
        sent.clear()
        Thread.sleep(400)
        assertEquals(listOf("MEDIA_LIST_REQUEST"), sentKinds())
    }
}
