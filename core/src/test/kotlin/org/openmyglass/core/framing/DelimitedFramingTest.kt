package org.openmyglass.core.framing

import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.assertThrows
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.InputStream

class DelimitedFramingTest {

    private fun frame(payload: ByteArray): ByteArray =
        ByteArrayOutputStream().also { DelimitedFraming.writeFrame(it, payload) }.toByteArray()

    /** Delivers at most one byte per read, like a slow Bluetooth socket. */
    private class OneByteAtATime(bytes: ByteArray) : InputStream() {
        private val inner = ByteArrayInputStream(bytes)
        override fun read(): Int = inner.read()
        override fun read(b: ByteArray, off: Int, len: Int): Int = inner.read(b, off, minOf(len, 1))
    }

    @Test
    fun `writes length as varint then payload`() {
        val small = byteArrayOf(10, 20, 30)
        assertArrayEquals(byteArrayOf(0x03, 10, 20, 30), frame(small))

        val large = ByteArray(300) { it.toByte() }
        val out = frame(large)
        assertArrayEquals(byteArrayOf(0xAC.toByte(), 0x02), out.copyOfRange(0, 2))
        assertArrayEquals(large, out.copyOfRange(2, out.size))
    }

    @Test
    fun `reads back what it wrote`() {
        val a = byteArrayOf(1, 2, 3)
        val b = ByteArray(200) { 7 }
        val input = ByteArrayInputStream(frame(a) + frame(b))
        assertArrayEquals(a, DelimitedFraming.readFrame(input))
        assertArrayEquals(b, DelimitedFraming.readFrame(input))
    }

    @Test
    fun `returns null on clean end of stream`() {
        assertNull(DelimitedFraming.readFrame(ByteArrayInputStream(ByteArray(0))))
    }

    @Test
    fun `reassembles a frame delivered one byte per read`() {
        val payload = ByteArray(300) { (it * 3).toByte() }
        assertArrayEquals(payload, DelimitedFraming.readFrame(OneByteAtATime(frame(payload))))
    }

    @Test
    fun `throws on truncated payload`() {
        val input = ByteArrayInputStream(byteArrayOf(10, 1, 2, 3, 4))
        assertThrows<FramingException> { DelimitedFraming.readFrame(input) }
    }

    @Test
    fun `throws on oversized length`() {
        val tooBig = DelimitedFraming.MAX_FRAME_BYTES + 1
        // varint of MAX_FRAME_BYTES + 1, followed by nothing: must fail before trying to read the payload
        val varint = ByteArrayOutputStream()
        var v = tooBig
        while (v and 0x7F.inv() != 0) { varint.write((v and 0x7F) or 0x80); v = v ushr 7 }
        varint.write(v)
        assertThrows<FramingException> { DelimitedFraming.readFrame(ByteArrayInputStream(varint.toByteArray())) }
    }

    @Test
    fun `throws on malformed varint`() {
        val input = ByteArrayInputStream(ByteArray(6) { 0xFF.toByte() })
        assertThrows<FramingException> { DelimitedFraming.readFrame(input) }
    }
}
