package org.openmyglass.android.service

import org.openmyglass.android.transport.LinkSettings

object BootPolicy {
    /** Reconnect after a reboot only if the user had connected (and not disconnected) and the permission holds. */
    fun startAtBoot(settings: LinkSettings, hasBluetoothPermission: Boolean) = settings.autoStart && hasBluetoothPermission
}
