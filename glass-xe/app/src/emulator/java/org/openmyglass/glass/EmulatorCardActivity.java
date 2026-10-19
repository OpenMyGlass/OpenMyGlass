package org.openmyglass.glass;

import android.app.Activity;
import android.graphics.Color;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.Gravity;
import android.view.View;
import android.view.WindowManager;
import android.widget.FrameLayout;
import android.widget.TextView;

import org.openmyglass.glass.bridge.BridgeServer;
import org.openmyglass.glass.nav.CardBinder;
import org.openmyglass.glass.nav.HandlerScheduler;
import org.openmyglass.glass.nav.IconCache;
import org.openmyglass.glass.nav.NavCardState;

/**
 * Emulator stand-in for the Glass LiveCard: runs the bridge server on {@link #PORT} and draws the same card
 * layout in a 16:9 area, so the whole phone → Glass path can be tested on a normal Android emulator.
 */
public final class EmulatorCardActivity extends Activity {
    public static final int PORT = 7825;

    private final Handler main = new Handler(Looper.getMainLooper());
    private BridgeServer server;
    private FrameLayout cardArea;
    private TextView idle;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(Color.DKGRAY);
        root.setFitsSystemWindows(true);
        cardArea = new FrameLayout(this);
        idle = new TextView(this);
        idle.setText("OpenMyGlass (emulator)\nWaiting for navigation on port " + PORT);
        idle.setTextColor(Color.WHITE);
        idle.setGravity(Gravity.CENTER);
        cardArea.addView(idle);
        root.addView(cardArea, cardLayoutParams());
        setContentView(root);

        NavCardState state = new NavCardState(new IconCache(64), new NavCardState.Listener() {
            @Override public void onChanged(final NavCardState.Card card) {
                main.post(new Runnable() { @Override public void run() { show(card); } });
            }
            @Override public void onEnded() {
                main.post(new Runnable() { @Override public void run() { show(null); } });
            }
        }, new HandlerScheduler(main), 30_000);
        server = new BridgeServer(PORT, state);
        server.start();
    }

    /** Largest 16:9 area that fits the screen (Glass is 640×360). */
    private FrameLayout.LayoutParams cardLayoutParams() {
        int w = getResources().getDisplayMetrics().widthPixels;
        int h = getResources().getDisplayMetrics().heightPixels;
        int width = Math.min(w, h * 16 / 9);
        return new FrameLayout.LayoutParams(width, width * 9 / 16, Gravity.CENTER);
    }

    @Override
    public void onConfigurationChanged(android.content.res.Configuration newConfig) {
        super.onConfigurationChanged(newConfig);
        cardArea.setLayoutParams(cardLayoutParams());
    }

    private void show(NavCardState.Card card) {
        cardArea.removeAllViews();
        if (card == null) {
            cardArea.addView(idle);
            return;
        }
        View view = CardBinder.bind(getPackageName(), card).apply(this, cardArea);
        cardArea.addView(view);
    }

    @Override
    protected void onDestroy() {
        if (server != null) server.close();
        super.onDestroy();
    }
}
