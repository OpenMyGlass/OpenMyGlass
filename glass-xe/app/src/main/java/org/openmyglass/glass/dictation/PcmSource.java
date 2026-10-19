package org.openmyglass.glass.dictation;

import android.media.AudioFormat;
import android.media.AudioRecord;
import android.media.MediaRecorder;

import java.io.Closeable;
import java.io.DataInputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;

/** 16-bit mono PCM for dictation: the microphone, or a WAV clip for end-to-end tests on the emulator. */
public abstract class PcmSource implements Closeable {
    public abstract int sampleRate();

    /** Reads up to {@code buffer.length} samples; -1 at the end (clips only). */
    public abstract int read(short[] buffer) throws IOException;

    @Override
    public abstract void close();

    /** The microphone at 16 kHz, or 44.1 kHz (the only rate API 19 guarantees) when 16 kHz is not supported. */
    public static PcmSource microphone() throws IOException {
        for (int rate : new int[] {16_000, 44_100}) {
            int min = AudioRecord.getMinBufferSize(rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT);
            if (min <= 0) continue;
            final AudioRecord record = new AudioRecord(MediaRecorder.AudioSource.VOICE_RECOGNITION, rate,
                    AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, Math.max(min, rate / 5 * 2));
            if (record.getState() != AudioRecord.STATE_INITIALIZED) {
                record.release();
                continue;
            }
            record.startRecording();
            final int sampleRate = rate;
            return new PcmSource() {
                @Override public int sampleRate() { return sampleRate; }
                @Override public int read(short[] buffer) { return record.read(buffer, 0, buffer.length); }
                @Override public void close() {
                    try {
                        record.stop();
                    } catch (IllegalStateException ignored) {
                        // not recording
                    }
                    record.release();
                }
            };
        }
        throw new IOException("no usable microphone format");
    }

    /** A PCM16 mono WAV file (canonical 44-byte header), played at real-time pace like a microphone. */
    public static PcmSource wav(File file) throws IOException {
        final DataInputStream in = new DataInputStream(new FileInputStream(file));
        byte[] header = new byte[44];
        in.readFully(header);
        final int rate = (header[24] & 0xff) | (header[25] & 0xff) << 8 | (header[26] & 0xff) << 16 | (header[27] & 0xff) << 24;
        return new PcmSource() {
            @Override public int sampleRate() { return rate; }
            @Override public int read(short[] buffer) throws IOException {
                byte[] bytes = new byte[buffer.length * 2];
                int n = readFully(in, bytes);
                if (n <= 0) return -1;
                int samples = n / 2;
                for (int i = 0; i < samples; i++) buffer[i] = (short) ((bytes[2 * i] & 0xff) | (bytes[2 * i + 1] << 8));
                try {
                    Thread.sleep(samples * 1000L / rate);
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                }
                return samples;
            }
            @Override public void close() {
                try {
                    in.close();
                } catch (IOException ignored) {
                    // closing
                }
            }
        };
    }

    private static int readFully(InputStream in, byte[] bytes) throws IOException {
        int total = 0;
        while (total < bytes.length) {
            int n = in.read(bytes, total, bytes.length - total);
            if (n < 0) break;
            total += n;
        }
        return total;
    }
}
