package org.openmyglass.android.transport

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.openmyglass.core.bridge.BridgeLink
import org.openmyglass.core.companion.CompanionConstants

class TransportTargetsTest {
    private val glass = "F8:8F:CA:00:11:22"
    private val bt = LinkSettings(BridgeMode.BLUETOOTH, glassAddress = glass)
    private val tcp = LinkSettings(BridgeMode.TCP, glassAddress = glass, tcpHost = "127.0.0.1", tcpPort = 7825)
    private val ready = Environment(bluetoothOn = true, hasBluetoothPermission = true)

    @Test
    fun `bluetooth mode targets the bridge uuid on the chosen glass`() {
        assertEquals(TransportTarget.Rfcomm(glass, BridgeLink.RFCOMM_UUID), TransportTargets.bridge(bt, ready))
    }

    @Test
    fun `tcp mode targets only tcp even with a glass chosen`() {
        // Review Focus 5: exactly one transport for the bridge
        assertEquals(TransportTarget.Tcp("127.0.0.1", 7825), TransportTargets.bridge(tcp, ready))
    }

    @Test
    fun `tcp mode works without bluetooth`() {
        assertEquals(TransportTarget.Tcp("127.0.0.1", 7825),
            TransportTargets.bridge(tcp, Environment(bluetoothOn = false, hasBluetoothPermission = false)))
    }

    @Test
    fun `bluetooth targets are unavailable when bluetooth is off, unpermitted or no glass is chosen`() {
        assertNull(TransportTargets.bridge(bt, ready.copy(bluetoothOn = false)))
        assertNull(TransportTargets.bridge(bt, ready.copy(hasBluetoothPermission = false)))
        assertNull(TransportTargets.bridge(bt.copy(glassAddress = null), ready))
        assertNull(TransportTargets.companion(bt, ready.copy(bluetoothOn = false)))
    }

    @Test
    fun `off mode has no bridge`() {
        assertNull(TransportTargets.bridge(bt.copy(bridgeMode = BridgeMode.OFF), ready))
    }

    @Test
    fun `companion always uses the firmware uuid over bluetooth`() {
        assertEquals(TransportTarget.Rfcomm(glass, CompanionConstants.SECURE_UUID), TransportTargets.companion(tcp, ready))
        assertNull(TransportTargets.companion(tcp.copy(companionEnabled = false), ready))
    }
}
