package org.openmyglass.companion

import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import java.io.File
import java.io.FileOutputStream

/** Records the microphone to a 16 kHz mono WAV file, the format of a Glass recording. */
class VoiceRecorder(private val file: File) {
    @Volatile private var running = false
    private var thread: Thread? = null
    private var startedAt = 0L

    /** The caller holds RECORD_AUDIO. */
    @SuppressLint("MissingPermission")
    fun start() {
        val minBuffer = AudioRecord.getMinBufferSize(RATE, CHANNEL, ENCODING)
        val record = AudioRecord(MediaRecorder.AudioSource.MIC, RATE, CHANNEL, ENCODING, minBuffer * 4)
        running = true
        startedAt = System.currentTimeMillis()
        thread = Thread {
            FileOutputStream(file).use { out ->
                out.write(Wav.placeholder())
                val buffer = ByteArray(minBuffer)
                record.startRecording()
                while (running) {
                    val n = record.read(buffer, 0, buffer.size)
                    if (n > 0) out.write(buffer, 0, n)
                }
                record.stop()
                record.release()
            }
            Wav.finish(file, RATE)
        }.apply { start() }
    }

    /** Stops and returns the length in milliseconds. */
    fun stop(): Int {
        running = false
        thread?.join()
        return (System.currentTimeMillis() - startedAt).toInt()
    }

    private companion object {
        const val RATE = 16000
        const val CHANNEL = AudioFormat.CHANNEL_IN_MONO
        const val ENCODING = AudioFormat.ENCODING_PCM_16BIT
    }
}
