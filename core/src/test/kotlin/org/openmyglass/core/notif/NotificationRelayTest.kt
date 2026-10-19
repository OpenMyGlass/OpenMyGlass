package org.openmyglass.core.notif

import kotlinx.coroutines.runBlocking
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.openmyglass.protocol.v1.Bridge
import org.openmyglass.protocol.v1.Bridge.Frame

class NotificationRelayTest {
    private val iconHex = "cd".repeat(32)
    private val actions = mutableListOf<Pair<String, NotificationRelay.ActionRequest>>()
    private val relay = NotificationRelay(
        filter = { NotificationFilter("org.openmyglass.android") },
        icons = { if (it == iconHex) byteArrayOf(1, 2) else null },
        onAction = { key, req -> actions += key to req },
    )
    private val sent = mutableListOf<Frame>()

    private fun item(key: String = "k1", text: String = "Ciao", ongoing: Boolean = false, icon: String? = null) = NotificationItem(
        key, "com.whatsapp", "WhatsApp", "Anna", text, 1L, NotificationItem.CATEGORY_MESSAGE, ongoing, false,
        NotificationItem.IMPORTANCE_HIGH, null, listOf(ItemAction("Reply", true), ItemAction("Read", false)), icon)

    private fun attach() = runBlocking { relay.attach { sent += it } }
    private fun action(key: String, index: Int? = null) = Bridge.NotificationAction.newBuilder().setKey(key).also {
        if (index == null) it.dismiss = true else it.actionIndex = index }.build()

    @Test
    fun `posted sends once and only on change`() = runBlocking {
        // Review Focus 1: a chat app re-posting the same notification must not resend it
        attach()
        relay.posted(item()); relay.posted(item()); relay.posted(item(text = "Ciao!"))
        assertEquals(listOf("Ciao", "Ciao!"), sent.filter { it.hasNotification() }.map { it.notification.notification.text })
    }

    @Test
    fun `removed sends removal for shown items only`() = runBlocking {
        attach()
        relay.posted(item("k1"))
        relay.removed("k1"); relay.removed("never-shown")
        assertEquals(listOf("k1"), sent.filter { it.hasNotificationRemoved() }.map { it.notificationRemoved.key })
    }

    @Test
    fun `filtered items never sent and their removal is silent`() = runBlocking {
        attach()
        relay.posted(item("k1", ongoing = true)); relay.removed("k1")
        assertEquals(0, sent.size)
    }

    @Test
    fun `an item that becomes filtered is removed from glass`() = runBlocking {
        attach()
        relay.posted(item("k1")); relay.posted(item("k1", ongoing = true))
        assertEquals(1, sent.count { it.hasNotificationRemoved() })
    }

    @Test
    fun `attach sends the full current set without waking`() = runBlocking {
        // Review Focus 3: after a reconnect Glass gets exactly the current set; old notifications must not wake it
        relay.posted(item("k1")); relay.posted(item("k2")); relay.removed("k1")
        attach()
        val posted = sent.filter { it.hasNotification() }.map { it.notification.notification }
        assertEquals(listOf("k2"), posted.map { it.key })
        assertTrue(posted.none { it.wake })
    }

    @Test
    fun `a new notification may wake`() = runBlocking {
        attach()
        relay.posted(item("k1"))
        assertTrue(sent.single { it.hasNotification() }.notification.notification.wake)
    }

    @Test
    fun `action for an unknown key is ignored`() = runBlocking {
        // Review Focus 2: dismissed on the phone while Glass showed the menu
        attach()
        relay.onAction(action("gone", 0))
        assertEquals(0, actions.size)
    }

    @Test
    fun `action and dismiss reach the callback`() = runBlocking {
        attach()
        relay.posted(item("k1"))
        relay.onAction(action("k1", 1)); relay.onAction(action("k1"))
        assertEquals(listOf("k1" to NotificationRelay.ActionRequest.Fire(1), "k1" to NotificationRelay.ActionRequest.Dismiss), actions)
    }

    @Test
    fun `app icon sent once per session before the first notification using it`() = runBlocking {
        attach()
        relay.posted(item("k1", icon = iconHex)); relay.posted(item("k2", icon = iconHex))
        assertEquals(listOf("icon", "n", "n"), sent.map { if (it.hasIcon()) "icon" else "n" })
        sent.clear(); attach()
        assertEquals(1, sent.count { it.hasIcon() })
    }
}
