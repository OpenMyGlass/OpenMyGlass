package org.openmyglass.core.transport

import java.io.Closeable
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream

enum class TransportKind { RFCOMM, TCP, BLE }

/** One open, bidirectional byte stream to the glasses. Closing it closes both directions. */
interface Channel : Closeable {
    val input: InputStream
    val output: OutputStream
}

/** Opens [Channel]s to the glasses. [connect] blocks; run it on `Dispatchers.IO`. */
interface Transport {
    val kind: TransportKind

    @Throws(IOException::class)
    fun connect(): Channel
}
