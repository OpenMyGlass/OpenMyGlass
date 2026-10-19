package org.openmyglass.glass.bridge;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * Varint32 length prefix + payload: the framing of the OpenMyGlass bridge (same as the phone's
 * DelimitedFraming in core). Plain Java for API 19.
 */
final class Framing {
    /** Bridge frames are capped at 64 KiB (BLE-friendly, architecture §10). */
    static final int MAX_FRAME_BYTES = 64 * 1024;

    private Framing() {}

    static void write(OutputStream out, byte[] payload) throws IOException {
        int value = payload.length;
        while ((value & ~0x7F) != 0) {
            out.write((value & 0x7F) | 0x80);
            value >>>= 7;
        }
        out.write(value);
        out.write(payload);
        out.flush();
    }

    /** Returns the next payload, or null on a clean end of stream before a frame starts. */
    static byte[] read(InputStream in) throws IOException {
        int first = in.read();
        if (first == -1) return null;
        int length = first & 0x7F;
        int b = first;
        int count = 1;
        while ((b & 0x80) != 0) {
            if (count == 5) throw new IOException("malformed varint length");
            b = in.read();
            if (b == -1) throw new IOException("truncated varint length");
            length |= (b & 0x7F) << (7 * count);
            count++;
        }
        if (length < 0 || length > MAX_FRAME_BYTES) throw new IOException("frame length " + length + " out of range");
        byte[] payload = new byte[length];
        int read = 0;
        while (read < length) {
            int n = in.read(payload, read, length - read);
            if (n == -1) throw new IOException("truncated frame");
            read += n;
        }
        return payload;
    }
}
