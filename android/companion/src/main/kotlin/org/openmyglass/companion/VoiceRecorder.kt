package org.openmyglass.companion

import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import java.io.File
import java.io.FileOutputStream
import java.io.IOException

/** Records the microphone to a 16 kHz mono WAV file, the format of a Glass recording. */
class VoiceRecorder(private val file: File) {
    @Volatile private var running = false

    @Volatile private var failed = false
    private var thread: Thread? = null
    private var startedAt = 0L

    /** The caller holds RECORD_AUDIO. False if the microphone cannot be had. */
    @SuppressLint("MissingPermission")
    fun start(): Boolean {
        val minBuffer = AudioRecord.getMinBufferSize(RATE, CHANNEL, ENCODING)
        if (minBuffer <= 0) return false
        val record = AudioRecord(MediaRecorder.AudioSource.MIC, RATE, CHANNEL, ENCODING, minBuffer * 4)
        if (record.state != AudioRecord.STATE_INITIALIZED) {
            record.release()
            return false
        }
        running = true
        startedAt = System.currentTimeMillis()
        thread = Thread {
            try {
                FileOutputStream(file).use { out ->
                    out.write(Wav.placeholder())
                    val buffer = ByteArray(minBuffer)
                    record.startRecording()
                    var heard = false
                    while (running) {
                        val n = record.read(buffer, 0, buffer.size)
                        // Below zero is an error that stays: the microphone was taken away.
                        if (n < 0) {
                            failed = true
                            break
                        }
                        if (n > 0) {
                            out.write(buffer, 0, n)
                            heard = true
                        }
                    }
                    if (!heard) failed = true
                }
                if (!failed) Wav.finish(file, RATE)
            } catch (_: IOException) {
                // Most likely a full disk.
                failed = true
            } finally {
                record.stop()
                record.release()
            }
        }.apply { start() }
        return true
    }

    /** Stops and returns the length in milliseconds, or null if the recording failed. */
    fun stop(): Int? {
        running = false
        thread?.join()
        thread = null
        return if (failed) null else (System.currentTimeMillis() - startedAt).toInt()
    }

    private companion object {
        const val RATE = 16000
        const val CHANNEL = AudioFormat.CHANNEL_IN_MONO
        const val ENCODING = AudioFormat.ENCODING_PCM_16BIT
    }
}
