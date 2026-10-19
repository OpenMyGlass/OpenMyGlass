package org.openmyglass.android.transport

import org.openmyglass.core.transport.Channel
import org.openmyglass.core.transport.Transport
import org.openmyglass.core.transport.TransportKind
import java.io.IOException

/** A [Transport] resolved on every connect, for links whose manager takes a fixed transport. */
class ProvidedTransport(override val kind: TransportKind, private val provide: () -> Transport?) : Transport {
    override fun connect(): Channel = (provide() ?: throw IOException("transport unavailable")).connect()
}
