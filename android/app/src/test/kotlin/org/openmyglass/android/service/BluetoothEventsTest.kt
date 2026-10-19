package org.openmyglass.android.service

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BluetoothEventsTest {
    private val glass = "F8:8F:CA:00:11:22"

    @Test
    fun `acl connected reconnects only for the configured glass`() {
        // Issue #17: a headset or car connecting must not wake the retries
        assertTrue(BluetoothEvents.shouldReconnect(BluetoothEvents.Event.AclConnected(glass), glass))
        assertTrue(BluetoothEvents.shouldReconnect(BluetoothEvents.Event.AclConnected(glass.lowercase()), glass))
        assertFalse(BluetoothEvents.shouldReconnect(BluetoothEvents.Event.AclConnected("00:11:22:33:44:55"), glass))
        assertFalse(BluetoothEvents.shouldReconnect(BluetoothEvents.Event.AclConnected(null), glass))
        assertFalse(BluetoothEvents.shouldReconnect(BluetoothEvents.Event.AclConnected(glass), null))
    }

    @Test
    fun `bluetooth turning on reconnects, turning off does not`() {
        assertTrue(BluetoothEvents.shouldReconnect(BluetoothEvents.Event.AdapterOn, glass))
        assertTrue(BluetoothEvents.shouldReconnect(BluetoothEvents.Event.AdapterOn, null))
        assertFalse(BluetoothEvents.shouldReconnect(BluetoothEvents.Event.Other, glass))
    }
}
