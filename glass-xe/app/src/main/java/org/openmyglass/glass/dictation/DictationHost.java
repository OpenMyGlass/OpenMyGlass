package org.openmyglass.glass.dictation;

import android.os.Handler;
import android.os.Looper;

/**
 * The process-wide dictation client (created by whoever runs the bridge server) and the screen currently showing
 * a dictation, which receives its events on the main thread.
 */
public final class DictationHost {
    private static final Handler MAIN = new Handler(Looper.getMainLooper());
    private static volatile DictationClient.Listener screen;

    public static final DictationClient CLIENT = new DictationClient(new DictationClient.Listener() {
        @Override public void onTranscript(final int requestId, final String text, final String error) {
            MAIN.post(new Runnable() { @Override public void run() {
                DictationClient.Listener s = screen;
                if (s != null) s.onTranscript(requestId, text, error);
            } });
        }
        @Override public void onResult(final int requestId, final boolean ok, final String message) {
            MAIN.post(new Runnable() { @Override public void run() {
                DictationClient.Listener s = screen;
                if (s != null) s.onResult(requestId, ok, message);
            } });
        }
        @Override public void onUnavailable() {
            MAIN.post(new Runnable() { @Override public void run() {
                DictationClient.Listener s = screen;
                if (s != null) s.onUnavailable();
            } });
        }
    });

    private DictationHost() {}

    public static void setScreen(DictationClient.Listener listener) {
        screen = listener;
    }
}
