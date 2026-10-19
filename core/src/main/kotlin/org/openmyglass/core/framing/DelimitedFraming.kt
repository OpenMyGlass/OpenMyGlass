package org.openmyglass.core.framing

import java.io.IOException
import java.io.InputStream
import java.io.OutputStream

/** A frame on the wire could not be decoded: truncated, oversized or with a malformed length. */
class FramingException(message: String) : IOException(message)

/**
 * Varint32 length prefix followed by the payload: the framing of the Glass companion protocol
 * (same as protobuf's writeDelimitedTo/parseDelimitedFrom), used on every OpenMyGlass link.
 */
object DelimitedFraming {
    const val MAX_FRAME_BYTES: Int = 4 * 1024 * 1024
    private const val MAX_VARINT_BYTES = 5

    fun writeFrame(out: OutputStream, payload: ByteArray) {
        var value = payload.size
        while (value and 0x7F.inv() != 0) {
            out.write((value and 0x7F) or 0x80)
            value = value ushr 7
        }
        out.write(value)
        out.write(payload)
        out.flush()
    }

    /** Returns the next payload, or `null` on a clean end of stream before the first byte of a frame. */
    fun readFrame(input: InputStream, maxBytes: Int = MAX_FRAME_BYTES): ByteArray? {
        val first = input.read()
        if (first == -1) return null
        val length = readVarint(first, input)
        if (length < 0 || length > maxBytes) throw FramingException("frame length $length exceeds $maxBytes")
        val payload = ByteArray(length)
        var read = 0
        while (read < length) {
            val n = input.read(payload, read, length - read)
            if (n == -1) throw FramingException("truncated frame: $read of $length bytes")
            read += n
        }
        return payload
    }

    private fun readVarint(first: Int, input: InputStream): Int {
        var result = first and 0x7F
        var b = first
        var count = 1
        while (b and 0x80 != 0) {
            if (count == MAX_VARINT_BYTES) throw FramingException("malformed varint length")
            b = input.read()
            if (b == -1) throw FramingException("truncated varint length")
            result = result or ((b and 0x7F) shl (7 * count))
            count++
        }
        return result
    }
}
