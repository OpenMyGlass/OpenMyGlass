package org.openmyglass.glass.bridge

import org.junit.Assert.assertEquals
import org.junit.Test
import org.openmyglass.protocol.v1.Bridge

class FanOutHandlerTest {
    @Test
    fun `every handler sees every event in order`() {
        val log = mutableListOf<String>()
        fun handler(name: String) = object : BridgeServer.FrameHandler {
            override fun onSessionStarted(sender: BridgeServer.Sender) { log += "$name start" }
            override fun onFrame(frame: Bridge.Frame) { log += "$name frame" }
            override fun onSessionEnded(sender: BridgeServer.Sender) { log += "$name end" }
        }
        val fan = FanOutHandler(listOf(handler("nav"), handler("notif")))
        val s = BridgeServer.Sender { }
        fan.onSessionStarted(s); fan.onFrame(Bridge.Frame.getDefaultInstance()); fan.onSessionEnded(s)
        assertEquals(listOf("nav start", "notif start", "nav frame", "notif frame", "nav end", "notif end"), log)
    }
}
