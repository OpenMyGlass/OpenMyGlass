package org.openmyglass.companion

import java.io.File
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** PCM 16-bit little-endian mono, the audio format the Glass and the speech engines share. */
class PcmAudio(val sampleRate: Int, val samples: ByteArray)

object Wav {
    private const val HEADER = 44

    /** Reads a PCM WAV file, downmixing to mono. Null if it is not 16-bit PCM. */
    fun parse(bytes: ByteArray): PcmAudio? {
        if (bytes.size < 12 || String(bytes, 0, 4, Charsets.US_ASCII) != "RIFF") return null
        val b = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        var channels = 0
        var rate = 0
        var bits = 0
        var offset = 12
        while (offset + 8 <= bytes.size) {
            val id = String(bytes, offset, 4, Charsets.US_ASCII)
            val size = b.getInt(offset + 4)
            val body = offset + 8
            if (size < 0) return null
            when (id) {
                "fmt " -> {
                    if (size < 16 || body + 16 > bytes.size) return null
                    if (b.getShort(body).toInt() != 1) return null
                    channels = b.getShort(body + 2).toInt()
                    rate = b.getInt(body + 4)
                    bits = b.getShort(body + 14).toInt()
                }
                "data" -> {
                    if (channels <= 0 || bits != 16 || rate <= 0) return null
                    val end = minOf(body.toLong() + size, bytes.size.toLong()).toInt()
                    return PcmAudio(rate, downmix(bytes, body, end, channels))
                }
            }
            offset = (body.toLong() + size + (size and 1)).coerceAtMost(Int.MAX_VALUE.toLong()).toInt()
        }
        return null
    }

    private fun downmix(bytes: ByteArray, start: Int, end: Int, channels: Int): ByteArray {
        if (channels == 1) return bytes.copyOfRange(start, end)
        val b = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        val frames = (end - start) / (2 * channels)
        val out = ByteBuffer.allocate(frames * 2).order(ByteOrder.LITTLE_ENDIAN)
        for (frame in 0 until frames) {
            var sum = 0
            for (channel in 0 until channels) {
                sum += b.getShort(start + (frame * channels + channel) * 2)
            }
            out.putShort((sum / channels).toShort())
        }
        return out.array()
    }

    /** Fills in the header of a file that started with [placeholder] and then got raw samples. */
    fun finish(file: File, sampleRate: Int) {
        RandomAccessFile(file, "rw").use { f ->
            val dataSize = (f.length() - HEADER).toInt()
            f.seek(0)
            f.write(header(sampleRate, dataSize))
        }
    }

    fun placeholder(): ByteArray = ByteArray(HEADER)

    fun header(sampleRate: Int, dataSize: Int): ByteArray =
        ByteBuffer.allocate(HEADER).order(ByteOrder.LITTLE_ENDIAN).apply {
            put("RIFF".toByteArray())
            putInt(36 + dataSize)
            put("WAVEfmt ".toByteArray())
            putInt(16)
            putShort(1) // PCM
            putShort(1) // mono
            putInt(sampleRate)
            putInt(sampleRate * 2)
            putShort(2)
            putShort(16)
            put("data".toByteArray())
            putInt(dataSize)
        }.array()
}
