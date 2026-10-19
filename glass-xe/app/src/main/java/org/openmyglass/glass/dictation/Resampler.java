package org.openmyglass.glass.dictation;

/** PCM helpers for dictation: the phone recognizer wants 16 kHz mono PCM16 little-endian. */
public final class Resampler {
    private Resampler() {}

    /** Downsamples by averaging each output sample's input window (cheap low-pass, enough for speech). */
    public static short[] downsample(short[] in, int inRate, int outRate) {
        if (inRate == outRate) return in.clone();
        int outLength = (int) ((long) in.length * outRate / inRate);
        short[] out = new short[outLength];
        double ratio = (double) inRate / outRate;
        for (int i = 0; i < outLength; i++) {
            int from = (int) (i * ratio);
            int to = Math.min(in.length, Math.max(from + 1, (int) ((i + 1) * ratio)));
            long sum = 0;
            for (int j = from; j < to; j++) sum += in[j];
            out[i] = (short) (sum / (to - from));
        }
        return out;
    }

    public static byte[] toBytes(short[] samples) {
        byte[] out = new byte[samples.length * 2];
        for (int i = 0; i < samples.length; i++) {
            out[2 * i] = (byte) (samples[i] & 0xff);
            out[2 * i + 1] = (byte) ((samples[i] >> 8) & 0xff);
        }
        return out;
    }
}
