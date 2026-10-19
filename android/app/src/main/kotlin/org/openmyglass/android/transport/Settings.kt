package org.openmyglass.android.transport

import android.content.Context

/** [LinkSettings] in SharedPreferences. */
class Settings(context: Context) {
    private val prefs = context.getSharedPreferences("links", Context.MODE_PRIVATE)

    fun load() = LinkSettings(
        bridgeMode = runCatching { BridgeMode.valueOf(prefs.getString(MODE, null)!!) }.getOrDefault(BridgeMode.BLUETOOTH),
        glassAddress = prefs.getString(GLASS, null),
        tcpHost = prefs.getString(HOST, null) ?: LinkSettings().tcpHost,
        tcpPort = prefs.getInt(PORT, LinkSettings().tcpPort),
        companionEnabled = prefs.getBoolean(COMPANION, true),
        autoStart = prefs.getBoolean(AUTO_START, false),
    )

    fun save(s: LinkSettings) = prefs.edit()
        .putString(MODE, s.bridgeMode.name).putString(GLASS, s.glassAddress)
        .putString(HOST, s.tcpHost).putInt(PORT, s.tcpPort).putBoolean(COMPANION, s.companionEnabled).putBoolean(AUTO_START, s.autoStart)
        .apply()

    private companion object {
        const val MODE = "bridge_mode"
        const val GLASS = "glass_address"
        const val HOST = "tcp_host"
        const val PORT = "tcp_port"
        const val COMPANION = "companion_enabled"
        const val AUTO_START = "auto_start"
    }
}
