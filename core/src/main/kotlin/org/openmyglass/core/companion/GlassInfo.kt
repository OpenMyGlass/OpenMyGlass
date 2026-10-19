package org.openmyglass.core.companion

import org.openmyglass.protocol.companion.Companion.Envelope

/** What the phone tells Glass about itself in the handshake. */
data class PhoneState(
    val sdkVersion: Int,
    val timeZoneId: String,
    val wifiConnected: Boolean,
    val interactive: Boolean,
    val gpsEnabled: Boolean,
    val photoSyncEnabled: Boolean = false,
    val keyboardTextEntrySupported: Boolean = false,
)

/** What Glass tells us in its handshake envelope. */
data class GlassHello(
    val remoteVersion: Int,
    val glassSdkVersion: Int?,
    val wallpaperFeatureEnabled: Boolean,
)

/** Device info from a `glassInfoResponseG2C`; fields absent on the wire are `null`. */
data class GlassInfo(
    val batteryLevel: Int?,
    val storageTotalBytes: Long?,
    val storageAvailableBytes: Long?,
    val deviceName: String?,
    val softwareVersion: String?,
    val needSetup: Boolean?,
    val screenWidthPx: Int?,
    val screenHeightPx: Int?,
)

sealed interface CompanionEvent {
    data class GlassInfoReceived(val info: GlassInfo) : CompanionEvent
    data class Other(val envelope: Envelope) : CompanionEvent
}
