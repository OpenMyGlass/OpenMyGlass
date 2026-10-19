package org.openmyglass.android.transport

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import org.openmyglass.core.transport.Channel
import org.openmyglass.core.transport.Transport
import org.openmyglass.core.transport.TransportKind
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.util.UUID

/** Secure RFCOMM to a bonded device (bonding authenticates both ends). Requires BLUETOOTH_CONNECT. */
class RfcommTransport(private val device: BluetoothDevice, private val uuid: UUID) : Transport {
    override val kind = TransportKind.RFCOMM

    @SuppressLint("MissingPermission")   // checked by TransportTargets before this transport is created
    override fun connect(): Channel {
        val socket = device.createRfcommSocketToServiceRecord(uuid)
        try {
            socket.connect()
        } catch (e: IOException) {
            runCatching { socket.close() }
            throw e
        }
        return object : Channel {
            override val input: InputStream = socket.inputStream
            override val output: OutputStream = socket.outputStream
            override fun close() = socket.close()
        }
    }
}
