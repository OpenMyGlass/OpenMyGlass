package org.openmyglass.android.service

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.openmyglass.android.transport.LinkSettings

class BootPolicyTest {
    @Test
    fun `starts at boot only if the user had connected and the permission is there`() {
        assertTrue(BootPolicy.startAtBoot(LinkSettings(autoStart = true), hasBluetoothPermission = true))
        assertFalse(BootPolicy.startAtBoot(LinkSettings(autoStart = false), hasBluetoothPermission = true))
        assertFalse(BootPolicy.startAtBoot(LinkSettings(autoStart = true), hasBluetoothPermission = false))
    }
}
