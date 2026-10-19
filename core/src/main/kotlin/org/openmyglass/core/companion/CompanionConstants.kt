package org.openmyglass.core.companion

import java.util.UUID

/** Constants of the original Glass companion protocol (see docs/protocol/transport.md and handshake.md). */
object CompanionConstants {
    /** RFCOMM service of the firmware's companion server (Glass listens, the phone connects). */
    val SECURE_UUID: UUID = UUID.fromString("F15CC914-E4BC-45CE-9930-CB7695385850")
    const val SOCKET_NAME = "Companion"

    /** Phone-side insecure RFCOMM service the firmware pings to make the companion reconnect. */
    val PING_SOCKET_UUID: UUID = UUID.fromString("DC935393-831A-4736-89E3-C8CE5BB20EEA")
    const val PING_SOCKET_NAME = "PingCompanion"

    const val MAJOR_VERSION = 6
    const val MINOR_VERSION = 10

    fun calculateVersion(major: Int, minor: Int): Int = (minor shl 16) + major
    fun majorOf(version: Int): Int = version and 0xFFFF
    fun minorOf(version: Int): Int = version ushr 16

    /**
     * The version we present. We present 6.10, the XE24 value: an equal major avoids the
     * firmware's OTA prompt (handshake.md).
     */
    val VERSION: Int = calculateVersion(MAJOR_VERSION, MINOR_VERSION)
}
