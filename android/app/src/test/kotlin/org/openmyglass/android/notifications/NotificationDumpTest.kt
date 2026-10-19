package org.openmyglass.android.notifications

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class NotificationDumpTest {

    private fun dump(extras: Map<String, Any?>, customContent: Boolean = false) = NotificationDump.dump(
        packageName = "com.google.android.apps.maps", postTime = 1L, category = "navigation", isOngoing = true,
        extras = extras, actionTitles = listOf("Exit navigation"),
        hasCustomContentView = customContent, hasCustomBigView = false,
    )

    @Suppress("UNCHECKED_CAST")
    private fun extrasOf(d: Map<String, Any?>) = d["extras"] as Map<String, Any?>

    @Test
    fun `strings and numbers pass through`() {
        val d = dump(mapOf("android.title" to "200 m", "android.progress" to 3, "android.showWhen" to true))
        assertEquals(mapOf("android.title" to "200 m", "android.progress" to 3, "android.showWhen" to true), extrasOf(d))
        assertEquals("com.google.android.apps.maps", d["package"])
        assertEquals(listOf("Exit navigation"), d["actions"])
    }

    @Test
    fun `char sequences become strings`() {
        val d = dump(mapOf("android.text" to StringBuilder("Via Roma")))
        assertEquals("Via Roma", extrasOf(d)["android.text"])
    }

    @Test
    fun `images become metadata only`() {
        val image = mapOf("type" to "image", "width" to 96, "height" to 96, "sha256" to "ab12")
        val d = dump(mapOf("android.largeIcon" to image, "raw" to byteArrayOf(1, 2, 3)))
        assertEquals(image, extrasOf(d)["android.largeIcon"])
        assertEquals(mapOf("type" to "byte[]"), extrasOf(d)["raw"])
        assertFalse(NotificationDump.toJson(d).contains("[1,2,3]"))
    }

    private class Foo

    @Test
    fun `unknown types become typed placeholders`() {
        assertEquals(mapOf("type" to "Foo"), extrasOf(dump(mapOf("x" to Foo())))["x"])
    }

    @Test
    fun `nested maps and lists are kept`() {
        val d = dump(mapOf("a" to mapOf("b" to listOf("x", 1)), "arr" to arrayOf("p", "q")))
        assertEquals(mapOf("b" to listOf("x", 1)), extrasOf(d)["a"])
        assertEquals(listOf("p", "q"), extrasOf(d)["arr"])
    }

    @Test
    fun `custom views are flagged`() {
        assertEquals(true, dump(emptyMap(), customContent = true)["hasCustomContentView"])
    }

    @Test
    fun `json encodes nested values and escapes strings`() {
        val json = NotificationDump.toJson(mapOf("t" to "a\"b\n", "n" to listOf(1, true, null)))
        assertEquals("""{"t":"a\"b\n","n":[1,true,null]}""", json)
    }
}
