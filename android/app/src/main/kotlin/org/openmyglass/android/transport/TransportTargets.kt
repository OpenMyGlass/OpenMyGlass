package org.openmyglass.android.transport

import org.openmyglass.core.bridge.BridgeLink
import org.openmyglass.core.companion.CompanionConstants
import java.util.UUID

enum class BridgeMode { BLUETOOTH, TCP, OFF }

/** What the user chose; persisted by [Settings]. */
data class LinkSettings(
    val bridgeMode: BridgeMode = BridgeMode.BLUETOOTH,
    /** MAC address of the bonded Glass. */
    val glassAddress: String? = null,
    /** Debug bridge over TCP (emulator, Wi-Fi). */
    val tcpHost: String = "127.0.0.1",
    val tcpPort: Int = 7825,
    /** Original companion link to the XE24 firmware (time zone, device info). */
    val companionEnabled: Boolean = true,
    /** The user connected and did not disconnect: start the service again after a reboot. */
    val autoStart: Boolean = false,
)

/** What the phone can do right now. */
data class Environment(val bluetoothOn: Boolean, val hasBluetoothPermission: Boolean)

sealed interface TransportTarget {
    data class Rfcomm(val address: String, val uuid: UUID) : TransportTarget
    data class Tcp(val host: String, val port: Int) : TransportTarget
}

/** Which transport each link uses; `null` means unavailable right now (the link manager backs off). */
object TransportTargets {
    fun bridge(settings: LinkSettings, env: Environment): TransportTarget? = when (settings.bridgeMode) {
        BridgeMode.OFF -> null
        BridgeMode.TCP -> TransportTarget.Tcp(settings.tcpHost, settings.tcpPort)
        BridgeMode.BLUETOOTH -> rfcomm(settings, env, BridgeLink.RFCOMM_UUID)
    }

    /** Why [bridge] gives no target, in words for the user; null when a target exists. */
    fun bridgeUnavailableReason(settings: LinkSettings, env: Environment): String? = when {
        settings.bridgeMode == BridgeMode.OFF -> "Bridge is off"
        settings.bridgeMode == BridgeMode.TCP -> null
        settings.glassAddress == null -> "Choose your Glass in settings"
        !env.hasBluetoothPermission -> "Bluetooth permission needed"
        !env.bluetoothOn -> "Bluetooth is off"
        else -> null
    }

    fun companion(settings: LinkSettings, env: Environment): TransportTarget? =
        if (settings.companionEnabled) rfcomm(settings, env, CompanionConstants.SECURE_UUID) else null

    private fun rfcomm(settings: LinkSettings, env: Environment, uuid: UUID): TransportTarget? {
        val address = settings.glassAddress ?: return null
        if (!env.bluetoothOn || !env.hasBluetoothPermission) return null
        return TransportTarget.Rfcomm(address, uuid)
    }
}
