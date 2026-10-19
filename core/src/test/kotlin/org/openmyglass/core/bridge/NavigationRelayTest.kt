package org.openmyglass.core.bridge

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.runBlocking
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test
import org.openmyglass.core.nav.Maneuver
import org.openmyglass.core.nav.NavUpdate
import org.openmyglass.core.nav.NavigationSource
import org.openmyglass.protocol.v1.Bridge
import org.openmyglass.protocol.v1.Bridge.Frame

class NavigationRelayTest {

    private class FakeSource(override val name: String) : NavigationSource {
        val flow = MutableStateFlow<NavUpdate?>(null)
        override val updates = flow
        override val available: StateFlow<Boolean> = MutableStateFlow(true)
    }

    private val osmand = FakeSource("osmand")
    private val gmaps = FakeSource("gmaps")
    private val iconHash = "ab".repeat(32)
    private val iconBytes = byteArrayOf(1, 2, 3)
    private val sent = mutableListOf<Frame>()
    private val relay = NavigationRelay(listOf(osmand, gmaps), icons = { if (it == iconHash) iconBytes else null },
        scope = CoroutineScope(Dispatchers.Unconfined)).also { r -> r.attach { sent += it } }

    private fun nav(distance: Int, source: String = "osmand", icon: String? = null) =
        NavUpdate(Maneuver.LEFT, distance, null, "Via Roma", "8:33 PM", source, icon)

    private fun navFrames() = sent.filter { it.hasNav() }.map { it.nav }

    @Test
    fun `sends nav state on change only`() {
        osmand.flow.value = nav(200)
        osmand.flow.value = nav(200)
        osmand.flow.value = nav(150)
        assertEquals(listOf(200, 150), navFrames().map { it.distanceM })
        assertEquals(Bridge.Maneuver.LEFT, navFrames().first().maneuver)
        assertEquals("Via Roma", navFrames().first().street)
    }

    @Test
    fun `prefers the first active source`() {
        gmaps.flow.value = nav(500, "gmaps")
        osmand.flow.value = nav(200, "osmand")
        gmaps.flow.value = nav(450, "gmaps")
        assertEquals(listOf("gmaps", "osmand"), navFrames().map { it.source })
    }

    @Test
    fun `falls back to the second source when the first ends`() {
        osmand.flow.value = nav(200)
        gmaps.flow.value = nav(500, "gmaps")
        osmand.flow.value = null
        assertEquals("gmaps", navFrames().last().source)
        assertEquals(0, sent.count { it.hasNavEnded() })
    }

    @Test
    fun `sends NavEnded once when all sources end`() {
        osmand.flow.value = nav(200)
        osmand.flow.value = null
        gmaps.flow.value = null
        assertEquals(1, sent.count { it.hasNavEnded() })
    }

    @Test
    fun `sends each icon once before its first reference`() {
        osmand.flow.value = nav(200, icon = iconHash)
        osmand.flow.value = nav(150, icon = iconHash)
        val iconIndex = sent.indexOfFirst { it.hasIcon() }
        val navIndex = sent.indexOfFirst { it.hasNav() }
        assertEquals(1, sent.count { it.hasIcon() })
        assert(iconIndex in 0 until navIndex) { "icon must precede its first reference" }
        assertEquals(iconHash, navFrames().first().iconSha256.toByteArray().joinToString("") { "%02x".format(it) })
    }

    @Test
    fun `resends state and icon after reattach`() {
        osmand.flow.value = nav(200, icon = iconHash)
        relay.detach()
        sent.clear()
        relay.attach { sent += it }
        assertEquals(1, sent.count { it.hasIcon() })
        assertEquals(listOf(200), navFrames().map { it.distanceM })
    }

    @Test
    fun `answers icon requests`() = runBlocking {
        relay.onIconRequest(ByteArray(32) { 0xab.toByte() })
        assertEquals(iconBytes.toList(), sent.single { it.hasIcon() }.icon.png.toByteArray().toList())
    }

    @Test
    fun `maneuvers map to the same proto names`() {
        Maneuver.entries.forEach { m ->
            val expected = if (m == Maneuver.UNKNOWN) "MANEUVER_UNKNOWN" else m.name
            assertEquals(expected, NavigationRelay.toProto(m).name)
        }
    }
}
