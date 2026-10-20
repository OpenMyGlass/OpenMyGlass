package org.openmyglass.core.bridge

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.openmyglass.core.connection.Backoff
import org.openmyglass.core.connection.LinkState
import org.openmyglass.core.framing.DelimitedFraming
import org.openmyglass.core.nav.Maneuver
import org.openmyglass.core.nav.NavUpdate
import org.openmyglass.core.nav.NavigationSource
import org.openmyglass.core.transport.TcpTransport
import org.openmyglass.protocol.v1.Bridge
import org.openmyglass.protocol.v1.Bridge.Frame
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import kotlin.time.Duration
import kotlin.time.Duration.Companion.milliseconds
import kotlin.time.Duration.Companion.seconds

class BridgeManagerTest {

    private class FakeSource : NavigationSource {
        override val name = "osmand"
        val flow = MutableStateFlow<NavUpdate?>(null)
        override val updates = flow
        override val available: StateFlow<Boolean> = MutableStateFlow(true)
    }

    /** Glass side of the bridge on a real TCP port: answers Hello, records every frame it receives. */
    private class GlassPeer : AutoCloseable {
        val server = ServerSocket(0)
        val port get() = server.localPort
        val frames = LinkedBlockingQueue<Frame>()
        @Volatile var socket: Socket? = null
        private val thread = Thread {
            while (!server.isClosed) {
                val s = runCatching { server.accept() }.getOrNull() ?: break
                socket = s
                runCatching {
                    while (true) {
                        val bytes = DelimitedFraming.readFrame(s.getInputStream()) ?: break
                        val frame = Frame.parseFrom(bytes)
                        if (frame.hasHello()) DelimitedFraming.writeFrame(s.getOutputStream(), Frame.newBuilder()
                            .setHello(Bridge.Hello.newBuilder().setProtocolVersion(1).addCapabilities("nav")).build().toByteArray())
                        else frames += frame
                    }
                }
                runCatching { s.close() }
            }
        }.apply { isDaemon = true; start() }
        fun next(): Frame? = frames.poll(3, TimeUnit.SECONDS)
        /** Next NavState, skipping the NavEnded the relay may send on attach before it has seen the source. */
        fun nextNav(): Frame { while (true) { val f = next()!!; if (f.hasNav()) return f } }
        fun dropConnection() = socket?.close()
        fun send(frame: Frame) = DelimitedFraming.writeFrame(socket!!.getOutputStream(), frame.toByteArray())
        override fun close() = server.close()
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val source = FakeSource()
    private val relay = NavigationRelay(listOf(source), icons = { null }, scope = scope)

    private val history = java.util.Collections.synchronizedList(mutableListOf<LinkState<Bridge.Hello>>())
    private val cursor = org.openmyglass.core.connection.HistoryCursor(history)

    private fun manager(port: Int, backoff: Backoff = Backoff(50.milliseconds, 200.milliseconds)) =
        BridgeManager({ TcpTransport("127.0.0.1", port) }, relay, appVersion = "test", scope = scope, backoff = backoff,
            onStateChange = { history += it })

    @Suppress("UnusedReceiverParameter")
    private fun BridgeManager.await(timeout: Duration = 3.seconds, p: (LinkState<Bridge.Hello>) -> Boolean) = cursor.await(timeout, p)

    private fun nav(d: Int) = NavUpdate(Maneuver.LEFT, d, null, "Via Roma", "20:33", "osmand", null)

    @AfterEach
    fun tearDown() = scope.cancel()

    @Test
    fun `connects and relays navigation`() {
        GlassPeer().use { glass ->
            val m = manager(glass.port); m.start()
            m.await { it is LinkState.Connected }
            assertTrue(glass.next()!!.hasNavEnded())   // idle announcement on attach
            source.flow.value = nav(120)
            assertEquals(120, glass.next()!!.nav.distanceM)
            m.stop()
        }
    }

    @Test
    fun `retries when the bridge app is missing`() {
        val closedPort = ServerSocket(0).use { it.localPort }
        val m = manager(closedPort, Backoff(20.milliseconds, 80.milliseconds)); m.start()
        val waits = (1..4).map { m.await { it is LinkState.WaitingToRetry } as LinkState.WaitingToRetry }
        assertTrue(waits.all { it.delay <= 80.milliseconds })
        m.stop()
    }

    @Test
    fun `reattaches after a drop`() {
        GlassPeer().use { glass ->
            source.flow.value = nav(500)
            val m = manager(glass.port); m.start()
            assertEquals(500, glass.nextNav().nav.distanceM)
            glass.dropConnection()
            // after reconnecting, the current state is sent again on the new session
            assertEquals(500, glass.nextNav().nav.distanceM)
            m.stop()
        }
    }

    @Test
    fun `notifications reach glass and actions from glass reach the relay`() {
        val chosen = java.util.concurrent.LinkedBlockingQueue<Pair<String, org.openmyglass.core.notif.NotificationRelay.ActionRequest>>()
        val notifications = org.openmyglass.core.notif.NotificationRelay(
            filter = { org.openmyglass.core.notif.NotificationFilter("own") }, icons = { null },
            onAction = { key, req -> chosen += key to req })
        GlassPeer().use { glass ->
            val m = BridgeManager({ TcpTransport("127.0.0.1", glass.port) }, relay, appVersion = "test", scope = scope,
                backoff = Backoff(50.milliseconds, 200.milliseconds), onStateChange = { history += it }, notifications = notifications)
            m.start()
            m.await { it is LinkState.Connected }
            runBlocking {
                notifications.posted(org.openmyglass.core.notif.NotificationItem("k1", "com.chat", "Chat", "Anna", "Ciao", 1L,
                    "msg", false, false, 4, null, emptyList(), null))
            }
            var f = glass.next()!!
            while (!f.hasNotification()) f = glass.next()!!
            assertEquals("k1", f.notification.notification.key)
            glass.send(Frame.newBuilder().setNotificationAction(Bridge.NotificationAction.newBuilder().setKey("k1").setDismiss(true)).build())
            assertEquals("k1" to org.openmyglass.core.notif.NotificationRelay.ActionRequest.Dismiss, chosen.poll(3, TimeUnit.SECONDS))
            m.stop()
        }
    }

    @Test
    fun `dictation frames from glass reach the dictation service`() {
        val dictation = org.openmyglass.core.dictation.DictationService(
            engine = { throw org.openmyglass.core.dictation.SpeechUnavailable("no model") },
            replies = { _, _, _ -> org.openmyglass.core.dictation.ReplyOutcome(true, "") }, scope = scope)
        GlassPeer().use { glass ->
            val m = BridgeManager({ TcpTransport("127.0.0.1", glass.port) }, relay, appVersion = "test", scope = scope,
                backoff = Backoff(50.milliseconds, 200.milliseconds), onStateChange = { history += it }, dictation = dictation)
            m.start()
            m.await { it is LinkState.Connected }
            glass.send(Frame.newBuilder().setDictationStart(Bridge.DictationStart.newBuilder().setRequestId(7).setSampleRate(16_000)).build())
            var f = glass.next()!!
            while (!f.hasTranscript()) f = glass.next()!!
            assertEquals(7, f.transcript.requestId)
            assertEquals("no model", f.transcript.error)
            m.stop()
        }
    }
}
