package org.openmyglass.android.service

import org.junit.Assert.assertEquals
import org.junit.Test
import org.openmyglass.core.connection.LinkState
import org.openmyglass.protocol.v1.Bridge
import kotlin.time.Duration.Companion.seconds

class ServiceStatusTest {
    private val hello = LinkState.Connected(Bridge.Hello.getDefaultInstance())

    @Test
    fun `missing bluetooth permission wins`() {
        // Review Focus 2
        assertEquals("Bluetooth permission needed", ServiceStatus.text(hello, companionConnected = true, permissionMissing = true))
    }

    @Test
    fun `connected bridge`() {
        assertEquals("Glass connected", ServiceStatus.text(hello, companionConnected = false, permissionMissing = false))
    }

    @Test
    fun `connecting`() {
        assertEquals("Connecting to Glass…", ServiceStatus.text(LinkState.Connecting, false, false))
        assertEquals("Connecting to Glass…", ServiceStatus.text(LinkState.Handshaking, false, false))
    }

    @Test
    fun `glass out of reach`() {
        // Review Focus 1
        assertEquals("Glass not reachable", ServiceStatus.text(LinkState.WaitingToRetry(3, 4.seconds), false, false))
        assertEquals("Glass not reachable", ServiceStatus.text(LinkState.Unavailable("bluetooth off"), false, false))
    }

    @Test
    fun `companion up but glass app missing`() {
        // Review Focus 3
        assertEquals("Glass connected, OpenMyGlass app not reachable",
            ServiceStatus.text(LinkState.WaitingToRetry(5, 60.seconds), companionConnected = true, permissionMissing = false))
    }

    @Test
    fun `an unavailable bridge shows the reason`() {
        // Issue #18: not "Glass not reachable" when the user just has to choose a Glass or turn Bluetooth on
        assertEquals("Choose your Glass in settings",
            ServiceStatus.text(LinkState.WaitingToRetry(1, 1.seconds), false, false, unavailableReason = "Choose your Glass in settings"))
        assertEquals("Glass connected", ServiceStatus.text(hello, false, false, unavailableReason = null))
    }
}
