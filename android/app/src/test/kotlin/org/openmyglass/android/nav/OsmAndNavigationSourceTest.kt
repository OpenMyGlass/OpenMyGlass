package org.openmyglass.android.nav

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.openmyglass.core.nav.Maneuver

class OsmAndNavigationSourceTest {

    private class FakeApi(private val navId: Long = 1, private val voiceId: Long = 2) : OsmAndApi {
        var nav: ((Int, Int) -> Unit)? = null
        var voice: ((List<String>) -> Unit)? = null
        val unregistered = mutableListOf<Long>()
        override fun registerNavigation(callback: (distanceTo: Int, turnType: Int) -> Unit): Long { nav = callback; return navId }
        override fun registerVoice(callback: (played: List<String>) -> Unit): Long { voice = callback; return voiceId }
        override fun unregister(id: Long) { unregistered += id }
    }

    private class FakeBinder(private val bindResult: Boolean = true) : OsmAndBinder {
        var onConnected: ((OsmAndApi) -> Unit)? = null
        var onDisconnected: (() -> Unit)? = null
        var unbound = false
        override fun bind(onConnected: (OsmAndApi) -> Unit, onDisconnected: () -> Unit): Boolean {
            this.onConnected = onConnected; this.onDisconnected = onDisconnected
            return bindResult
        }
        override fun unbind() { unbound = true }
    }

    @Test
    fun `bind failure means unavailable`() {
        val source = OsmAndNavigationSource(FakeBinder(bindResult = false))
        source.start()
        assertFalse(source.available.value)
        assertNull(source.current.value)
    }

    @Test
    fun `direction callbacks become updates`() {
        val binder = FakeBinder(); val api = FakeApi()
        val source = OsmAndNavigationSource(binder)
        source.start(); binder.onConnected!!(api)
        assertTrue(source.available.value)
        api.nav!!(300, 5)
        val update = source.current.value!!
        assertEquals(Maneuver.RIGHT, update.maneuver)
        assertEquals(300, update.distanceMeters)
        assertEquals("osmand", update.source)
        api.voice!!(listOf("Turn right onto Via Roma"))
        assertEquals("Turn right onto Via Roma", source.current.value!!.instruction)
    }

    @Test
    fun `service death re-registers after reconnect`() {
        val binder = FakeBinder(); val api1 = FakeApi(); val api2 = FakeApi()
        val source = OsmAndNavigationSource(binder)
        source.start(); binder.onConnected!!(api1); api1.nav!!(100, 2)
        binder.onDisconnected!!()
        assertFalse(source.available.value)
        assertNull(source.current.value)
        binder.onConnected!!(api2)
        assertTrue(source.available.value)
        assertTrue(api2.nav != null && api2.voice != null)
    }

    @Test
    fun `stop unregisters and unbinds`() {
        val binder = FakeBinder(); val api = FakeApi()
        val source = OsmAndNavigationSource(binder)
        source.start(); binder.onConnected!!(api)
        source.stop()
        assertEquals(listOf(1L, 2L), api.unregistered.sorted())
        assertTrue(binder.unbound)
        assertFalse(source.available.value)
        assertNull(source.current.value)
    }

    @Test
    fun `refused registration means not authorized`() {
        // OsmAnd returns -1 when the app is not enabled in its "Connected apps" settings
        val binder = FakeBinder(); val api = FakeApi(navId = -1, voiceId = -1)
        val source = OsmAndNavigationSource(binder)
        source.start(); binder.onConnected!!(api)
        assertFalse(source.available.value)
        assertTrue(source.needsAuthorization.value)
        source.stop()
        assertTrue(api.unregistered.isEmpty())
    }
}
