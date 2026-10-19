package org.openmyglass.glass.nav

import com.google.protobuf.ByteString
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.openmyglass.glass.bridge.BridgeServer
import org.openmyglass.protocol.v1.Bridge

class NavCardStateTest {

    private val changes = mutableListOf<NavCardState.Card>()
    private var ended = 0
    private val sent = mutableListOf<Bridge.Frame>()
    private val pending = mutableListOf<Runnable>()
    private val scheduler = NavCardState.Scheduler { task, _ -> pending += task; NavCardState.Cancellable { pending -= task } }
    private val state = NavCardState(IconCache(8), object : NavCardState.Listener {
        override fun onChanged(card: NavCardState.Card) { changes += card }
        override fun onEnded() { ended++ }
    }, scheduler, 30_000)
    private val sender = BridgeServer.Sender { sent += it }

    private val sha = ByteString.copyFrom(ByteArray(32) { 0x11 })
    private val png = byteArrayOf(9, 8, 7)

    private fun nav(distance: Int, icon: ByteString? = null) = Bridge.Frame.newBuilder().setNav(
        Bridge.NavState.newBuilder().setManeuver(Bridge.Maneuver.LEFT).setDistanceM(distance).setStreet("Landstrasse")
            .setEta("8:33 PM").setSource("osmand").also { b -> icon?.let { b.setIconSha256(it) } }
    ).build()

    private fun ended() = Bridge.Frame.newBuilder().setNavEnded(Bridge.NavEnded.getDefaultInstance()).build()
    private fun icon() = Bridge.Frame.newBuilder().setIcon(Bridge.Icon.newBuilder().setSha256(sha).setPng(ByteString.copyFrom(png))).build()

    @Test
    fun `nav state produces a card`() {
        state.onSessionStarted(sender)
        state.onFrame(nav(200))
        val card = changes.single()
        assertEquals(Bridge.Maneuver.LEFT, card.maneuver)
        assertEquals(200, card.distanceM)
        assertEquals("Landstrasse", card.street)
        assertEquals("8:33 PM", card.eta)
        assertNull(card.iconPng)
    }

    @Test
    fun `nav ended removes the card`() {
        state.onSessionStarted(sender)
        state.onFrame(nav(200))
        state.onFrame(ended())
        assertEquals(1, ended)
    }

    @Test
    fun `unknown icon is requested once and the card updates when it arrives`() {
        state.onSessionStarted(sender)
        state.onFrame(nav(200, sha))
        state.onFrame(nav(150, sha))
        assertEquals(1, sent.count { it.hasIconRequest() && it.iconRequest.sha256 == sha })
        assertNull(changes.last().iconPng)
        state.onFrame(icon())
        assertArrayEquals(png, changes.last().iconPng)
        assertEquals(150, changes.last().distanceM)
    }

    @Test
    fun `reconnect without nav ended replaces the state`() {
        state.onSessionStarted(sender)
        state.onFrame(nav(200))
        state.onSessionEnded()
        state.onSessionStarted(sender)
        state.onFrame(nav(120))
        pending.toList().forEach { it.run() }   // a cancelled grace timer must not fire
        assertEquals(0, ended)
        assertEquals(120, changes.last().distanceM)
    }

    @Test
    fun `session loss ends the card after the grace period`() {
        state.onSessionStarted(sender)
        state.onFrame(nav(200))
        state.onSessionEnded()
        assertEquals(0, ended)
        pending.toList().forEach { it.run() }
        assertEquals(1, ended)
    }
}
