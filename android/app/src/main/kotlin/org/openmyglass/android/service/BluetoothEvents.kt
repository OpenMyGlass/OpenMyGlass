package org.openmyglass.android.service

/** Which Bluetooth broadcasts should skip the links' retry wait (§6: only events that can change reachability). */
object BluetoothEvents {
    sealed interface Event {
        data object AdapterOn : Event
        data class AclConnected(val address: String?) : Event
        data object Other : Event
    }

    fun shouldReconnect(event: Event, glassAddress: String?): Boolean = when (event) {
        Event.AdapterOn -> true
        is Event.AclConnected -> glassAddress != null && event.address.equals(glassAddress, ignoreCase = true)
        Event.Other -> false
    }
}
