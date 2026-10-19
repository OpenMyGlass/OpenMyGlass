package org.openmyglass.glass.bridge

import org.junit.Assert.assertEquals
import org.junit.Test

class BridgeChannelsTest {
    @Test
    fun `tcp listens only on debuggable builds`() {
        val rfcomm = object : ServerChannel {
            override fun open() {}
            override fun accept(): ServerChannel.Connection = throw java.io.IOException()
            override fun close() {}
        }
        assertEquals(listOf(rfcomm), BridgeChannels.forBuild(rfcomm, 7825, false))
        val debug = BridgeChannels.forBuild(rfcomm, 7825, true)
        assertEquals(2, debug.size)
        assertEquals(TcpServerChannel::class.java, debug[1].javaClass)
    }
}
