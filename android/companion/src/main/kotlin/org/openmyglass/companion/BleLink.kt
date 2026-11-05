package org.openmyglass.companion

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothStatusCodes
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.ParcelUuid
import android.os.SystemClock
import org.openmyglass.corebindings.BleWritePacer
import org.openmyglass.corebindings.BleWriteStep
import org.openmyglass.corebindings.LinkProfile
import org.openmyglass.corebindings.bleService
import org.openmyglass.corebindings.linkProfileConstrained
import java.io.IOException
import java.util.UUID
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.Semaphore
import java.util.concurrent.TimeUnit

/**
 * The everyday link: the Glass's GATT service as a byte stream. Each write and notification is the
 * next run of bytes, so the core cannot tell this from TCP. Only radio calls live here. The spacing
 * of the writes and the service's identifiers are in the core (`omg_core::ble`), for iOS to share.
 *
 * It holds no reference to a screen, so a foreground service can own it as it is:
 * see docs/companion-mobile.md.
 *
 * [write] blocks while the pacer spaces the bytes, which is the backpressure on the core. Reading
 * happens on Binder threads and never waits for a write.
 */
@SuppressLint("MissingPermission")
class BleLink private constructor(private val cancelled: () -> Boolean) : ByteLink {
    override val profile: LinkProfile = linkProfileConstrained()

    /** The address of the Glass at the other end. */
    var address: String? = null
        private set

    private val incoming = LinkedBlockingQueue<ByteArray>()
    private val steps = LinkedBlockingQueue<Step>()
    private val writeSlot = Semaphore(1)
    private val pacer = BleWritePacer()
    @Volatile private var gatt: BluetoothGatt? = null
    @Volatile private var rx: BluetoothGattCharacteristic? = null
    private var leftover: ByteArray? = null

    @Volatile private var alive = false

    private sealed class Step {
        data class ConnectionChanged(val up: Boolean, val status: Int) : Step()
        data class Mtu(val mtu: Int) : Step()
        data class Services(val status: Int) : Step()
        data class Subscribed(val status: Int) : Step()
    }

    private val callback = object : BluetoothGattCallback() {
        override fun onConnectionStateChange(g: BluetoothGatt, status: Int, state: Int) {
            val up = state == BluetoothGatt.STATE_CONNECTED
            alive = up
            steps.add(Step.ConnectionChanged(up, status))
            if (!up) {
                // Whoever waits for a read or a write sees the end.
                incoming.add(END)
                writeSlot.release()
            }
        }

        override fun onMtuChanged(g: BluetoothGatt, mtu: Int, status: Int) {
            steps.add(Step.Mtu(mtu))
        }

        override fun onServicesDiscovered(g: BluetoothGatt, status: Int) {
            steps.add(Step.Services(status))
        }

        override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) {
            steps.add(Step.Subscribed(status))
        }

        override fun onCharacteristicWrite(
            g: BluetoothGatt,
            c: BluetoothGattCharacteristic,
            status: Int,
        ) {
            writeSlot.release()
        }

        override fun onCharacteristicChanged(
            g: BluetoothGatt,
            c: BluetoothGattCharacteristic,
            value: ByteArray,
        ) {
            if (value.isNotEmpty()) incoming.add(value)
        }

        // Before Android 13 the stack calls this one.
        @Deprecated("Deprecated in Android 13")
        @Suppress("DEPRECATION")
        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic) {
            c.value?.takeIf { it.isNotEmpty() }?.let { incoming.add(it.copyOf()) }
        }
    }

    private fun connect(context: Context, device: BluetoothDevice) {
        val service = bleService()
        val g = device.connectGatt(context, false, callback, BluetoothDevice.TRANSPORT_LE)
            ?: throw IOException("Bluetooth did not start a connection")
        gatt = g
        address = device.address
        // A connection attempt ends by itself in about 30 s, with a status.
        val up = await<Step.ConnectionChanged>(CONNECT_TIMEOUT_MS)
        if (!up.up) throw IOException("the connection failed, status ${up.status}")

        // The size first: at the default of 23 every later step takes much longer.
        mustStart(g.requestMtu(service.mtu.toInt()), "requestMtu")
        pacer.setMtu(await<Step.Mtu>(STEP_TIMEOUT_MS).mtu.toUShort())

        mustStart(g.discoverServices(), "discoverServices")
        val found = await<Step.Services>(STEP_TIMEOUT_MS)
        if (found.status != BluetoothGatt.GATT_SUCCESS) {
            throw IOException("discovery failed, status ${found.status}")
        }
        val gattService = g.getService(UUID.fromString(service.serviceUuid))
            ?: throw IOException("this device has no OpenMyGlass service")
        rx = gattService.getCharacteristic(UUID.fromString(service.rxUuid))
            ?: throw IOException("no rx")
        val tx = gattService.getCharacteristic(UUID.fromString(service.txUuid))
            ?: throw IOException("no tx")

        // The Glass starts the session on this write, and its first bytes wait in `incoming`.
        mustStart(g.setCharacteristicNotification(tx, true), "setCharacteristicNotification")
        val cccd = tx.getDescriptor(UUID.fromString(service.cccdUuid))
            ?: throw IOException("no descriptor")
        writeDescriptor(g, cccd)
        val subscribed = await<Step.Subscribed>(STEP_TIMEOUT_MS)
        if (subscribed.status != BluetoothGatt.GATT_SUCCESS) {
            throw IOException("subscribing failed, status ${subscribed.status}")
        }
    }

    @Suppress("DEPRECATION")
    private fun writeDescriptor(g: BluetoothGatt, cccd: BluetoothGattDescriptor) {
        val value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
        val started = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            g.writeDescriptor(cccd, value) == BluetoothStatusCodes.SUCCESS
        } else {
            cccd.value = value
            g.writeDescriptor(cccd)
        }
        mustStart(started, "writeDescriptor")
    }

    private fun mustStart(started: Boolean, what: String) {
        if (!started) throw IOException("$what did not start")
    }

    private inline fun <reified T : Step> await(timeoutMs: Long): T {
        val end = SystemClock.elapsedRealtime() + timeoutMs
        while (true) {
            if (cancelled()) throw IOException("cancelled")
            val left = end - SystemClock.elapsedRealtime()
            if (left <= 0) throw IOException("timed out waiting for ${T::class.simpleName}")
            val step = steps.poll(minOf(left, CANCEL_POLL_MS), TimeUnit.MILLISECONDS) ?: continue
            if (step is T) return step
            if (step is Step.ConnectionChanged && !step.up) {
                throw IOException("the link dropped during setup, status ${step.status}")
            }
        }
    }

    override fun read(buffer: ByteArray): Int {
        val chunk = leftover ?: incoming.take()
        if (chunk === END) {
            incoming.add(END)
            return -1
        }
        val n = minOf(chunk.size, buffer.size)
        System.arraycopy(chunk, 0, buffer, 0, n)
        leftover = if (n < chunk.size) chunk.copyOfRange(n, chunk.size) else null
        return n
    }

    override fun write(bytes: ByteArray) {
        pacer.push(bytes)
        while (true) {
            if (!alive) throw IOException("the link is gone")
            when (val step = pacer.nextStep(SystemClock.elapsedRealtime().toULong())) {
                is BleWriteStep.Idle -> return
                is BleWriteStep.Wait -> Thread.sleep(step.ms.toLong())
                is BleWriteStep.Write -> writeRun(step.bytes)
            }
        }
    }

    /** One write without response, and at most one the stack has not called back for. */
    private fun writeRun(run: ByteArray) {
        val g = gatt ?: throw IOException("not connected")
        val c = rx ?: throw IOException("not connected")
        while (true) {
            if (!writeSlot.tryAcquire(WRITE_CALLBACK_TIMEOUT_MS, TimeUnit.MILLISECONDS)) {
                throw IOException("the Bluetooth stack stopped taking writes")
            }
            if (!alive) throw IOException("the link is gone")
            if (startWrite(g, c, run)) return
            // The stack is busy: not an error, it has a moment's work to do first.
            writeSlot.release()
            Thread.sleep(STACK_BUSY_MS)
        }
    }

    @Suppress("DEPRECATION")
    private fun startWrite(
        g: BluetoothGatt,
        c: BluetoothGattCharacteristic,
        run: ByteArray,
    ): Boolean {
        val type = BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            g.writeCharacteristic(c, run, type) == BluetoothStatusCodes.SUCCESS
        } else {
            c.writeType = type
            c.value = run
            g.writeCharacteristic(c)
        }
    }

    override fun close() {
        alive = false
        incoming.add(END)
        writeSlot.release()
        gatt?.let {
            it.disconnect()
            it.close()
        }
        gatt = null
    }

    companion object {
        /** Marks the end of the stream in the read queue. Compared by identity. */
        private val END = ByteArray(0)

        private const val SCAN_TIMEOUT_MS = 20_000L
        private const val CONNECT_TIMEOUT_MS = 40_000L
        private const val STEP_TIMEOUT_MS = 15_000L
        private const val CANCEL_POLL_MS = 250L

        /** Stalls of 3.6 s were measured, so this is well above them. */
        private const val WRITE_CALLBACK_TIMEOUT_MS = 15_000L
        private const val STACK_BUSY_MS = 5L

        /** A stack that has just stopped scanning connects more reliably after a moment. */
        private const val AFTER_SCAN_MS = 300L

        /** What is still to be asked of the owner. */
        fun missingPermissions(context: Context): List<String> {
            val needed = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                listOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT)
            } else {
                listOf(Manifest.permission.ACCESS_FINE_LOCATION)
            }
            return needed.filter {
                context.checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED
            }
        }

        /**
         * Scans for the Glass and connects. Blocks, for up to a minute, so it is called from the
         * link's own thread. With [known] set, only that address will do: a phone with a bond is
         * after its own Glass.
         */
        @Throws(IOException::class)
        fun open(context: Context, known: String?, cancelled: () -> Boolean): BleLink {
            val link = BleLink(cancelled)
            try {
                if (missingPermissions(context).isNotEmpty()) {
                    throw IOException("Bluetooth permissions not granted")
                }
                val device = scan(context, known, cancelled)
                Thread.sleep(AFTER_SCAN_MS)
                link.connect(context, device)
                return link
            } catch (e: IOException) {
                link.close()
                throw e
            } catch (e: SecurityException) {
                link.close()
                throw IOException(e)
            } catch (e: IllegalArgumentException) {
                link.close()
                throw IOException(e)
            }
        }

        private fun scan(
            context: Context,
            known: String?,
            cancelled: () -> Boolean,
        ): BluetoothDevice {
            val adapter = context.getSystemService(BluetoothManager::class.java)?.adapter
            if (adapter == null || !adapter.isEnabled) throw IOException("Bluetooth is off")
            val scanner = adapter.bluetoothLeScanner ?: throw IOException("Bluetooth is off")
            val found = LinkedBlockingQueue<BluetoothDevice>()
            val failure = LinkedBlockingQueue<Int>()
            val callback = object : ScanCallback() {
                // By address here, not with a filter: the Glass's is random, and a filter on an
                // address may look for a public one.
                override fun onScanResult(type: Int, result: ScanResult) {
                    if (known == null || result.device.address == known) found.add(result.device)
                }

                override fun onScanFailed(code: Int) {
                    failure.add(code)
                }
            }
            val filter = ScanFilter.Builder()
                .setServiceUuid(ParcelUuid(UUID.fromString(bleService().serviceUuid)))
                .build()
            val settings = ScanSettings.Builder()
                .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
                .build()
            scanner.startScan(listOf(filter), settings, callback)
            try {
                val end = SystemClock.elapsedRealtime() + SCAN_TIMEOUT_MS
                while (SystemClock.elapsedRealtime() < end) {
                    if (cancelled()) throw IOException("cancelled")
                    failure.poll()?.let { throw IOException("scan failed, code $it") }
                    found.poll(CANCEL_POLL_MS, TimeUnit.MILLISECONDS)?.let { return it }
                }
                throw IOException("no Glass found")
            } finally {
                scanner.stopScan(callback)
            }
        }
    }
}
