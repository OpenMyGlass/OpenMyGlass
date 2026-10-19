package org.openmyglass.android.notifications

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.openmyglass.core.notif.ItemAction

class NotificationMapperTest {
    private val base = NotificationFields(
        key = "0|com.whatsapp|1|null|10159", packageName = "com.whatsapp", appName = "WhatsApp", postTime = 5L,
        category = "msg", ongoing = false, groupSummary = false, importance = 4, channelId = "chats",
        title = "Anna", conversationTitle = null, text = "Ci vediamo?", bigText = null, lastMessage = null,
        actions = listOf(ActionFields("Reply", hasRemoteInput = true), ActionFields("Mark as read", hasRemoteInput = false)),
        iconSha256 = "ab".repeat(32),
    )

    @Test
    fun `plain notification maps title text and actions`() {
        val item = NotificationMapper.map(base)
        assertEquals("Anna", item.title)
        assertEquals("Ci vediamo?", item.text)
        assertEquals(listOf(ItemAction("Reply", true), ItemAction("Mark as read", false)), item.actions)
        assertEquals(4, item.importance)
        assertEquals("ab".repeat(32), item.iconSha256)
    }

    @Test
    fun `big text wins over the collapsed text`() {
        assertEquals("long text", NotificationMapper.map(base.copy(bigText = "long text")).text)
    }

    @Test
    fun `messaging style uses the conversation title and the last message`() {
        val item = NotificationMapper.map(base.copy(title = null, conversationTitle = "Famiglia", text = null, lastMessage = "Papà: arrivo"))
        assertEquals("Famiglia", item.title)
        assertEquals("Papà: arrivo", item.text)
    }

    @Test
    fun `missing title and text stay null`() {
        // Review Focus 4: the filter drops empty ones, the model keeps nulls
        val item = NotificationMapper.map(base.copy(title = null, text = null))
        assertNull(item.title); assertNull(item.text)
    }

    @Test
    fun `blank action titles become empty and positions are kept`() {
        val item = NotificationMapper.map(base.copy(actions = listOf(ActionFields(" ", false), ActionFields("Archive", false))))
        assertEquals(listOf("", "Archive"), item.actions.map { it.title })
        assertFalse(item.actions[1].reply)
        assertTrue(item.actions.size == 2)
    }
}
