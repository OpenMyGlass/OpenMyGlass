package org.openmyglass.glass.dictation;

import android.app.Activity;
import android.graphics.Color;
import android.os.Bundle;
import android.os.Handler;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.Menu;
import android.view.MenuItem;
import android.view.View;
import android.view.WindowManager;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;

import java.io.File;
import java.io.IOException;
import java.util.concurrent.Executor;
import java.util.concurrent.Executors;

/**
 * Dictated reply to a notification: records, waits for the phone's offline transcript, then the owner chooses
 * Send, Retry or Cancel (tap opens the menu on Glass; the emulator shows buttons). Back cancels at any time.
 */
public final class DictationActivity extends Activity implements DictationClient.Listener {
    public static final String EXTRA_KEY = "key";
    public static final String EXTRA_ACTION = "action";
    /** Debug: a WAV file to dictate from instead of the microphone (emulator tests). */
    public static final String EXTRA_CLIP = "clip";
    /** Show on-screen buttons (the emulator has no touchpad). */
    public static final String EXTRA_BUTTONS = "buttons";

    private static final int SEND = 1;
    private static final int RETRY = 2;
    private static final int CANCEL = 3;
    private static final Executor IO = Executors.newSingleThreadExecutor();

    private enum Phase { RECORDING, TRANSCRIBING, REVIEW, FAILED, SENDING, DONE }

    private final Handler main = new Handler();
    private TextView status;
    private TextView transcript;
    private LinearLayout buttons;
    private Phase phase;
    private int requestId = -1;
    private DictationRecorder recorder;
    private String text;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setBackgroundColor(Color.BLACK);
        root.setPadding(40, 40, 40, 40);
        status = new TextView(this);
        status.setTextColor(0xFF999999);
        status.setTextSize(22);
        transcript = new TextView(this);
        transcript.setTextColor(Color.WHITE);
        transcript.setTextSize(32);
        buttons = new LinearLayout(this);
        buttons.setGravity(Gravity.CENTER_HORIZONTAL);
        root.addView(status);
        root.addView(transcript, new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1));
        root.addView(buttons);
        setContentView(root);
        DictationHost.setScreen(this);
        record();
    }

    private void record() {
        text = null;
        transcript.setText("");
        String clip = getIntent().getStringExtra(EXTRA_CLIP);
        final PcmSource source;
        try {
            source = clip != null ? PcmSource.wav(new File(clip)) : PcmSource.microphone();
        } catch (IOException e) {
            fail("Microphone not available");
            return;
        }
        show(Phase.RECORDING, "Speak now", null);
        final String key = getIntent().getStringExtra(EXTRA_KEY);
        final int action = getIntent().getIntExtra(EXTRA_ACTION, 0);
        IO.execute(new Runnable() { @Override public void run() {
            final int id = DictationHost.CLIENT.start(key, action, DictationRecorder.RATE);
            main.post(new Runnable() { @Override public void run() {
                if (id < 0) {
                    source.close();
                    fail("Phone not reachable");
                    return;
                }
                requestId = id;
                recorder = new DictationRecorder();
                recorder.start(source, DictationHost.CLIENT, id, new DictationRecorder.Callback() {
                    @Override public void onStopped(boolean heardSpeech) {
                        main.post(new Runnable() { @Override public void run() {
                            if (phase == Phase.RECORDING) show(Phase.TRANSCRIBING, "Transcribing on the phone…", null);
                        } });
                    }
                    @Override public void onFailed(final String message) {
                        main.post(new Runnable() { @Override public void run() { fail(message); } });
                    }
                });
            } });
        } });
    }

    private void show(Phase next, String statusText, String transcriptText) {
        phase = next;
        status.setText(statusText);
        if (transcriptText != null) transcript.setText(transcriptText);
        buttons.removeAllViews();
        if (!getIntent().getBooleanExtra(EXTRA_BUTTONS, false)) return;
        if (next == Phase.RECORDING) addButton("Stop", new Runnable() { @Override public void run() { stopRecording(); } });
        if (next == Phase.REVIEW) addButton("Send", new Runnable() { @Override public void run() { send(); } });
        if (next == Phase.REVIEW || next == Phase.FAILED) addButton("Retry", new Runnable() { @Override public void run() { record(); } });
        if (next != Phase.DONE && next != Phase.SENDING) addButton("Cancel", new Runnable() { @Override public void run() { cancel(); } });
    }

    private void addButton(String label, final Runnable action) {
        Button b = new Button(this);
        b.setText(label);
        b.setOnClickListener(new View.OnClickListener() { @Override public void onClick(View v) { action.run(); } });
        buttons.addView(b);
    }

    private void fail(String message) {
        show(Phase.FAILED, message + " · tap to retry", null);
    }

    private void stopRecording() {
        if (recorder != null) recorder.stop();
    }

    private void send() {
        if (phase != Phase.REVIEW || text == null) return;
        show(Phase.SENDING, "Sending…", null);
        final int id = requestId;
        IO.execute(new Runnable() { @Override public void run() { DictationHost.CLIENT.confirm(id, true); } });
    }

    private void cancel() {
        final int id = requestId;
        if (recorder != null) recorder.stop();
        if (id > 0) IO.execute(new Runnable() { @Override public void run() {
            DictationHost.CLIENT.cancel(id);
            DictationHost.CLIENT.confirm(id, false);
        } });
        finish();
    }

    @Override
    public void onTranscript(int id, String newText, String error) {
        if (id != requestId) return;
        if (error != null && error.length() > 0) {
            fail(error);
            return;
        }
        text = newText;
        show(Phase.REVIEW, "Tap to send", newText);
    }

    @Override
    public void onResult(int id, boolean ok, String message) {
        if (id != requestId) return;
        show(Phase.DONE, ok ? "Sent" : message, null);
        main.postDelayed(new Runnable() { @Override public void run() { finish(); } }, 1_500);
    }

    @Override
    public void onUnavailable() {
        if (phase != Phase.DONE) fail("Phone not reachable");
    }

    @Override
    public boolean onKeyDown(int keyCode, KeyEvent event) {
        if (keyCode == KeyEvent.KEYCODE_DPAD_CENTER || keyCode == KeyEvent.KEYCODE_ENTER) {
            if (phase == Phase.RECORDING) stopRecording();
            else if (phase == Phase.REVIEW) openOptionsMenu();
            else if (phase == Phase.FAILED) record();
            return true;
        }
        return super.onKeyDown(keyCode, event);
    }

    @Override
    public void onBackPressed() {
        cancel();
    }

    @Override
    public boolean onCreateOptionsMenu(Menu menu) {
        return true;
    }

    @Override
    public boolean onPrepareOptionsMenu(Menu menu) {
        menu.clear();
        if (phase != Phase.REVIEW) return false;
        menu.add(Menu.NONE, SEND, 0, "Send");
        menu.add(Menu.NONE, RETRY, 1, "Retry");
        menu.add(Menu.NONE, CANCEL, 2, "Cancel");
        return true;
    }

    @Override
    public boolean onOptionsItemSelected(MenuItem item) {
        switch (item.getItemId()) {
            case SEND: send(); return true;
            case RETRY: record(); return true;
            case CANCEL: cancel(); return true;
            default: return super.onOptionsItemSelected(item);
        }
    }

    @Override
    protected void onDestroy() {
        if (recorder != null) recorder.stop();
        DictationHost.setScreen(null);
        super.onDestroy();
    }
}
