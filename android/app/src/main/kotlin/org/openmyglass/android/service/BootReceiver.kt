package org.openmyglass.android.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import org.openmyglass.android.transport.Settings

/** Restarts the connection service after a reboot (BOOT_COMPLETED may start a connectedDevice service). */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return
        if (BootPolicy.startAtBoot(Settings(context).load(), OpenMyGlassService.hasBluetoothPermission(context))) {
            runCatching { OpenMyGlassService.start(context) }
        }
    }
}
