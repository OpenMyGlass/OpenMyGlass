package org.openmyglass.android.transport

import android.Manifest
import android.bluetooth.BluetoothManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import org.openmyglass.core.transport.TcpTransport
import org.openmyglass.core.transport.Transport

/** Turns a [TransportTarget] into a [Transport] for this device. */
object Transports {
    fun environment(context: Context): Environment {
        val permitted = Build.VERSION.SDK_INT < 31 ||
            context.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED
        val adapter = context.getSystemService(BluetoothManager::class.java)?.adapter
        return Environment(bluetoothOn = adapter?.isEnabled == true, hasBluetoothPermission = permitted)
    }

    fun create(context: Context, target: TransportTarget?): Transport? = when (target) {
        null -> null
        is TransportTarget.Tcp -> TcpTransport(target.host, target.port)
        is TransportTarget.Rfcomm -> context.getSystemService(BluetoothManager::class.java)?.adapter
            ?.let { runCatching { it.getRemoteDevice(target.address) }.getOrNull() }
            ?.let { RfcommTransport(it, target.uuid) }
    }
}
