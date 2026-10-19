package org.openmyglass.core.notif

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.openmyglass.protocol.v1.Bridge

class NotificationFilterTest {
    private val filter = NotificationFilter(ownPackage = "org.openmyglass.android", muted = setOf("com.spammy"))

    private fun item(
        pkg: String = "com.whatsapp", title: String? = "Anna", text: String? = "Ci vediamo alle 8?",
        category: String? = NotificationItem.CATEGORY_MESSAGE, ongoing: Boolean = false, summary: Boolean = false,
        importance: Int = NotificationItem.IMPORTANCE_HIGH, channel: String? = null,
        actions: List<ItemAction> = listOf(ItemAction("Reply", reply = true), ItemAction("Mark as read", reply = false)),
    ) = NotificationItem("0|$pkg|1|null|10", pkg, "App", title, text, 1_000L, category, ongoing, summary, importance, channel, actions, null)

    @Test
    fun `a normal message is accepted`() = assertTrue(filter.accept(item()))

    @Test
    fun `ongoing and group summaries are filtered`() {
        // Review Focus 1: navigation, music, progress and foreground-service notifications are ongoing
        assertFalse(filter.accept(item(ongoing = true)))
        assertFalse(filter.accept(item(summary = true)))
    }

    @Test
    fun `muted apps are filtered`() = assertFalse(filter.accept(item(pkg = "com.spammy")))

    @Test
    fun `own app filtered except the test channel`() {
        assertFalse(filter.accept(item(pkg = "org.openmyglass.android")))
        assertTrue(filter.accept(item(pkg = "org.openmyglass.android", channel = NotificationFilter.TEST_CHANNEL)))
    }

    @Test
    fun `empty notifications are filtered`() = assertFalse(filter.accept(item(title = null, text = " ")))

    @Test
    fun `low importance filtered unless messaging or call`() {
        assertFalse(filter.accept(item(category = null, importance = NotificationItem.IMPORTANCE_LOW)))
        assertTrue(filter.accept(item(category = NotificationItem.CATEGORY_MESSAGE, importance = NotificationItem.IMPORTANCE_LOW)))
        assertTrue(filter.accept(item(category = null, importance = NotificationItem.IMPORTANCE_DEFAULT)))
    }

    @Test
    fun `wake only for configured categories and when enabled`() {
        assertTrue(filter.wake(item(category = NotificationItem.CATEGORY_MESSAGE)))
        assertTrue(filter.wake(item(category = NotificationItem.CATEGORY_CALL)))
        assertFalse(filter.wake(item(category = "promo")))
        assertFalse(NotificationFilter("x", wakeEnabled = false).wake(item()))
    }
}

class NotificationItemTest {
    private fun item(title: String?, text: String?, actions: Int = 2) = NotificationItem(
        "k", "com.app", "App", title, text, 42L, null, false, false, NotificationItem.IMPORTANCE_HIGH, null,
        List(actions) { ItemAction("Action $it", reply = it == 0) }, "ab".repeat(32))

    @Test
    fun `proto carries fields, actions with their index and the reply flag`() {
        val p = item("Anna", "Ciao").toProto(wake = true)
        assertEquals("k", p.key); assertEquals("App", p.appName); assertEquals("Anna", p.title); assertEquals("Ciao", p.text)
        assertEquals(42L, p.postedAtMs); assertTrue(p.wake); assertEquals(32, p.iconSha256.size())
        assertEquals(listOf(0, 1), p.actionsList.map { it.index }); assertTrue(p.actionsList[0].reply); assertFalse(p.actionsList[1].reply)
    }

    @Test
    fun `missing title and text become empty`() {
        // Review Focus 4
        val p = item(null, null, actions = 0).toProto(wake = false)
        assertEquals("", p.title); assertEquals("", p.text); assertEquals(0, p.actionsCount)
    }

    @Test
    fun `long text truncated on code points`() {
        val emoji = "😀".repeat(1_500)                       // surrogate pairs: must not be split
        val p = item("t".repeat(500), emoji).toProto(wake = false)
        assertEquals(NotificationItem.TITLE_MAX, p.title.codePointCount(0, p.title.length))
        assertEquals(NotificationItem.TEXT_MAX, p.text.codePointCount(0, p.text.length))
        assertTrue(p.text.endsWith("…"))
        assertFalse(Character.isHighSurrogate(p.text[p.text.length - 2]))
    }

    @Test
    fun `at most three actions`() = assertEquals(3, item("a", "b", actions = 6).toProto(false).actionsCount)

    @Test
    fun `frame for worst-case item stays under 64 KiB`() {
        val worst = item("𝕏".repeat(10_000), "𝕏".repeat(100_000), actions = 10)
            .copy(actions = List(10) { ItemAction("𝕏".repeat(5_000), true) }, appName = "𝕏".repeat(5_000))
        val frame = Bridge.Frame.newBuilder().setNotification(Bridge.NotificationPosted.newBuilder().setNotification(worst.toProto(true))).build()
        assertTrue(frame.serializedSize < 64 * 1024, "frame is ${frame.serializedSize} bytes")
    }
}
