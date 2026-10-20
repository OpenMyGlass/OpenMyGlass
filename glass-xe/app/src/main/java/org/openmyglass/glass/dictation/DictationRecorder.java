package org.openmyglass.glass.dictation;

import java.io.IOException;

/**
 * Records one dictation and streams it as 16 kHz PCM16 in ~100 ms chunks. The microphone is open only while
 * recording (battery, architecture §6); it stops on silence, at the maximum length, or when asked.
 */
public final class DictationRecorder {

    public interface Callback {
        /** Recording ended; {@code heardSpeech} false if nothing above the silence threshold was captured. */
        void onStopped(boolean heardSpeech);
        void onFailed(String message);
    }

    public static final int RATE = 16_000;

    private volatile boolean stopRequested;
    private Thread thread;

    public void start(final PcmSource source, final DictationClient client, final int requestId, final Callback callback) {
        thread = new Thread(new Runnable() {
            @Override public void run() {
                SilenceDetector detector = new SilenceDetector(RATE, 2_000, 30_000);
                short[] buffer = new short[source.sampleRate() / 10];
                try {
                    while (!stopRequested) {
                        int n = source.read(buffer);
                        if (n < 0) break;
                        if (n == 0) continue;
                        short[] chunk = new short[n];
                        System.arraycopy(buffer, 0, chunk, 0, n);
                        short[] pcm = Resampler.downsample(chunk, source.sampleRate(), RATE);
                        if (!client.audio(requestId, Resampler.toBytes(pcm))) {
                            callback.onFailed("Phone not reachable");
                            return;
                        }
                        if (detector.feed(pcm)) break;
                    }
                    client.finish(requestId);
                    callback.onStopped(detector.heardSpeech());
                } catch (IOException e) {
                    client.cancel(requestId);
                    callback.onFailed("Microphone error");
                } finally {
                    source.close();
                }
            }
        }, "dictation");
        thread.start();
    }

    /** Ends the recording now (the owner tapped); the audio so far is transcribed. */
    public void stop() {
        stopRequested = true;
    }
}
