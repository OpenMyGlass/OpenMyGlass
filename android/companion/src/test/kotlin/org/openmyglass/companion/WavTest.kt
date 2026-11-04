package org.openmyglass.companion

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

class WavTest {
    @Test
    fun readsWhatItWrites() {
        val samples = byteArrayOf(1, 2, 3, 4)
        val audio = Wav.parse(Wav.header(16000, samples.size) + samples)!!
        assertEquals(16000, audio.sampleRate)
        assertArrayEquals(samples, audio.samples)
    }

    @Test
    fun skipsChunksItDoesNotKnow() {
        val header = Wav.header(8000, 2)
        val list = "LIST".toByteArray() + byteArrayOf(3, 0, 0, 0, 9, 9, 9, 0)
        val wav = header.copyOfRange(0, 36) + list + header.copyOfRange(36, 44) + byteArrayOf(5, 6)
        assertArrayEquals(byteArrayOf(5, 6), Wav.parse(wav)!!.samples)
    }

    @Test
    fun downmixesStereo() {
        val b = ByteBuffer.allocate(44 + 4).order(ByteOrder.LITTLE_ENDIAN)
        b.put(Wav.header(8000, 4))
        b.putShort(22, 2) // two channels
        b.position(44)
        b.putShort(100)
        b.putShort(300)
        val mono = Wav.parse(b.array())!!
        assertEquals(2, mono.samples.size)
        assertEquals(200, ByteBuffer.wrap(mono.samples).order(ByteOrder.LITTLE_ENDIAN).short.toInt())
    }

    @Test
    fun refusesWhatIsNotPcm16() {
        assertNull(Wav.parse(ByteArray(100)))
        val eightBit = Wav.header(8000, 2) + byteArrayOf(1, 2)
        ByteBuffer.wrap(eightBit).order(ByteOrder.LITTLE_ENDIAN).putShort(34, 8)
        assertNull(Wav.parse(eightBit))
    }
}
