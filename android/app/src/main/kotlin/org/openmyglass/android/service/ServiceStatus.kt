package org.openmyglass.android.service

import org.openmyglass.core.connection.LinkState

/** The one line the foreground notification shows. */
object ServiceStatus {
    fun text(bridge: LinkState<*>, companionConnected: Boolean, permissionMissing: Boolean, unavailableReason: String? = null): String = when {
        permissionMissing -> "Bluetooth permission needed"
        bridge is LinkState.Connected<*> -> "Glass connected"
        unavailableReason != null -> unavailableReason
        bridge == LinkState.Connecting || bridge == LinkState.Handshaking -> "Connecting to Glass…"
        bridge == LinkState.Stopped -> "Stopped"
        companionConnected -> "Glass connected, OpenMyGlass app not reachable"
        else -> "Glass not reachable"
    }
}
