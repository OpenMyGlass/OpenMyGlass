package org.openmyglass.glass.dictation;

/**
 * Decides when to stop recording (battery, architecture §6): after speech followed by {@code silenceMs} of
 * silence, when nothing was said within {@code noSpeechMs}, or at {@code maxMs}. Energy threshold on RMS.
 */
public final class SilenceDetector {
    private static final double SPEECH_RMS = 500;

    private final int sampleRate;
    private final int silenceMs;
    private final int maxMs;
    private final int noSpeechMs;
    private boolean speech;
    private double totalMs;
    private double silentMs;

    public SilenceDetector(int sampleRate, int silenceMs, int maxMs) {
        this(sampleRate, silenceMs, maxMs, 8_000);
    }

    public SilenceDetector(int sampleRate, int silenceMs, int maxMs, int noSpeechMs) {
        this.sampleRate = sampleRate;
        this.silenceMs = silenceMs;
        this.maxMs = maxMs;
        this.noSpeechMs = noSpeechMs;
    }

    /** Feeds one buffer; true when recording should stop. */
    public boolean feed(short[] samples) {
        if (samples.length == 0) return false;
        double sum = 0;
        for (short s : samples) sum += (double) s * s;
        double rms = Math.sqrt(sum / samples.length);
        double ms = samples.length * 1000.0 / sampleRate;
        totalMs += ms;
        if (rms >= SPEECH_RMS) {
            speech = true;
            silentMs = 0;
        } else {
            silentMs += ms;
        }
        return totalMs >= maxMs || (speech && silentMs >= silenceMs) || (!speech && totalMs >= noSpeechMs);
    }

    public boolean heardSpeech() {
        return speech;
    }
}
