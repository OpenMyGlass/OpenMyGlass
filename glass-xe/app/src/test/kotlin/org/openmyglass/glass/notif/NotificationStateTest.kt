package org.openmyglass.glass.notif

import com.google.protobuf.ByteString
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.openmyglass.glass.bridge.BridgeServer
import org.openmyglass.glass.nav.IconCache
import org.openmyglass.protocol.v1.Bridge

class NotificationStateTest {
    private val lists = mutableListOf<List<NotificationState.Item>>()
    private val news = mutableListOf<Pair<String, Boolean>>()
    private val sent = mutableListOf<Bridge.Frame>()
    private val sender = BridgeServer.Sender { sent += it }
    private val timers = mutableListOf<Runnable>()
    private val state = NotificationState(IconCache(16), object : NotificationState.Listener {
        override fun onChanged(items: List<NotificationState.Item>) { lists += items }
        override fun onNew(item: NotificationState.Item, wake: Boolean) { news += item.key to wake }
    }, 20, org.openmyglass.glass.nav.NavCardState.Scheduler { task, _ -> timers += task; org.openmyglass.glass.nav.NavCardState.Cancellable { timers -= task } })

    private fun posted(key: String, text: String = "Ciao", wake: Boolean = true, icon: ByteString? = null) =
        Bridge.Frame.newBuilder().setNotification(Bridge.NotificationPosted.newBuilder().setNotification(
            Bridge.Notification.newBuilder().setKey(key).setAppName("Chat").setTitle("Anna").setText(text).setWake(wake)
                .addActions(Bridge.NotificationActionInfo.newBuilder().setIndex(1).setTitle("Read"))
                .also { b -> icon?.let { b.setIconSha256(it) } })).build()

    private fun removed(key: String) = Bridge.Frame.newBuilder().setNotificationRemoved(Bridge.NotificationRemoved.newBuilder().setKey(key)).build()
    private fun keys() = lists.last().map { it.key }

    @Test
    fun `posted adds newest first`() {
        state.onSessionStarted(sender)
        state.onFrame(posted("a")); state.onFrame(posted("b"))
        assertEquals(listOf("b", "a"), keys())
        assertEquals(listOf("a" to true, "b" to true), news)
    }

    @Test
    fun `update replaces in place without onNew`() {
        state.onSessionStarted(sender)
        state.onFrame(posted("a")); state.onFrame(posted("b")); state.onFrame(posted("a", text = "Ciao!"))
        assertEquals(listOf("b", "a"), keys())
        assertEquals("Ciao!", lists.last()[1].text)
        assertEquals(2, news.size)
    }

    @Test
    fun `removed removes`() {
        state.onSessionStarted(sender)
        state.onFrame(posted("a")); state.onFrame(removed("a"))
        assertEquals(emptyList<String>(), keys())
    }

    @Test
    fun `new session replaces the whole set`() {
        // Review Focus 3: the phone resends its current set on every session
        state.onSessionStarted(sender)
        state.onFrame(posted("a")); state.onFrame(posted("b"))
        state.onSessionStarted(sender)
        state.onFrame(posted("b", wake = false))
        assertEquals(listOf("b"), keys())
    }

    @Test
    fun `cap drops the oldest`() {
        state.onSessionStarted(sender)
        repeat(25) { state.onFrame(posted("k$it")) }
        assertEquals(20, lists.last().size)
        assertEquals("k24", lists.last().first().key)
        assertEquals("k5", lists.last().last().key)
    }

    @Test
    fun `action on a removed key is not sent`() {
        // Review Focus 2: removed on the phone while the menu was open
        state.onSessionStarted(sender)
        state.onFrame(posted("a")); state.onFrame(removed("a"))
        assertFalse(state.requestAction("a", 1))
        assertFalse(state.dismiss("a"))
        assertEquals(0, sent.size)
    }

    @Test
    fun `action is sent with the phone index`() {
        state.onSessionStarted(sender)
        state.onFrame(posted("a"))
        assertTrue(state.requestAction("a", 1))
        assertEquals(1, sent.single().notificationAction.actionIndex)
    }

    @Test
    fun `dismiss sends and removes locally`() {
        state.onSessionStarted(sender)
        state.onFrame(posted("a"))
        assertTrue(state.dismiss("a"))
        assertTrue(sent.single().notificationAction.dismiss)
        assertEquals(emptyList<String>(), keys())
    }

    @Test
    fun `no session means actions are not sent`() {
        state.onSessionStarted(sender)
        state.onFrame(posted("a"))
        state.onSessionEnded(sender)
        assertFalse(state.requestAction("a", 1))
    }

    @Test
    fun `unknown app icon is requested once and fills in when it arrives`() {
        val sha = ByteString.copyFrom(ByteArray(32) { 7 })
        state.onSessionStarted(sender)
        state.onFrame(posted("a", icon = sha)); state.onFrame(posted("b", icon = sha))
        assertEquals(1, sent.count { it.hasIconRequest() })
        state.onFrame(Bridge.Frame.newBuilder().setIcon(Bridge.Icon.newBuilder().setSha256(sha).setPng(ByteString.copyFrom(byteArrayOf(1)))).build())
        assertTrue(lists.last().all { it.iconPng != null })
    }

    @Test
    fun `a dismiss the phone confirms removes the card for good`() {
        state.onSessionStarted(sender)
        state.onFrame(posted("a")); state.dismiss("a")
        state.onFrame(removed("a"))
        timers.toList().forEach { it.run() }
        assertEquals(emptyList<String>(), keys())
    }

    @Test
    fun `a dismiss the phone does not confirm brings the card back`() {
        // Issue #47: FLAG_NO_CLEAR or no listener on the phone
        state.onSessionStarted(sender)
        state.onFrame(posted("a")); state.dismiss("a")
        assertEquals(emptyList<String>(), keys())
        timers.toList().forEach { it.run() }
        assertEquals(listOf("a"), keys())
    }
}
