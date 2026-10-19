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
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;

import java.util.Arrays;
import java.util.List;

import org.openmyglass.glass.bridge.BridgeServer;
import org.openmyglass.glass.bridge.FanOutHandler;
import org.openmyglass.glass.notif.NotificationCards;
import org.openmyglass.glass.notif.NotificationCommands;
import org.openmyglass.glass.notif.NotificationState;
import org.openmyglass.glass.nav.CardBinder;
import org.openmyglass.glass.nav.HandlerScheduler;
import org.openmyglass.glass.nav.IconCache;
import org.openmyglass.glass.nav.NavCardState;

/**
 * Emulator stand-in for the Glass LiveCards: runs the bridge server on {@link #PORT} and draws the same card
 * layouts in a 16:9 area, so the whole phone → Glass path can be tested on a normal Android emulator. The
 * navigation card wins while navigating; otherwise the selected notification is shown, with a button bar
 * standing in for the Glass menu (its actions, Dismiss, Next).
 */
public final class EmulatorCardActivity extends Activity {
    public static final int PORT = 7825;

    private final Handler main = new Handler(Looper.getMainLooper());
    private BridgeServer server;
    private FrameLayout cardArea;
    private TextView idle;
    private LinearLayout menuBar;
    private NotificationState notifications;
    private NavCardState.Card navCard;
    private int selected;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(Color.DKGRAY);
        root.setFitsSystemWindows(true);
        cardArea = new FrameLayout(this);
        idle = new TextView(this);
        idle.setText("OpenMyGlass (emulator)\nWaiting for navigation and notifications on port " + PORT);
        idle.setTextColor(Color.WHITE);
        idle.setGravity(Gravity.CENTER);
        cardArea.addView(idle);
        root.addView(cardArea, cardLayoutParams());
        menuBar = new LinearLayout(this);
        menuBar.setOrientation(LinearLayout.HORIZONTAL);
        root.addView(menuBar, new FrameLayout.LayoutParams(FrameLayout.LayoutParams.WRAP_CONTENT,
                FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM | Gravity.CENTER_HORIZONTAL));
        setContentView(root);

        IconCache icons = new IconCache(64);
        NavCardState state = new NavCardState(icons, new NavCardState.Listener() {
            @Override public void onChanged(final NavCardState.Card card) {
                main.post(new Runnable() { @Override public void run() { navCard = card; render(); } });
            }
            @Override public void onEnded() {
                main.post(new Runnable() { @Override public void run() { navCard = null; render(); } });
            }
        }, new HandlerScheduler(main), 30_000);
        notifications = new NotificationState(icons, new NotificationState.Listener() {
            @Override public void onChanged(List<NotificationState.Item> items) {
                main.post(new Runnable() { @Override public void run() { render(); } });
            }
            @Override public void onNew(NotificationState.Item item, boolean wake) {
                main.post(new Runnable() { @Override public void run() { selected = 0; render(); } });
            }
        }, 20);
        server = new BridgeServer(PORT, new FanOutHandler(Arrays.<BridgeServer.FrameHandler>asList(state, notifications)));
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

    private void render() {
        cardArea.removeAllViews();
        menuBar.removeAllViews();
        if (navCard != null) {
            cardArea.addView(CardBinder.bind(getPackageName(), navCard).apply(this, cardArea));
            return;
        }
        final List<NotificationState.Item> items = notifications.items();
        if (items.isEmpty()) {
            cardArea.addView(idle);
            return;
        }
        selected = Math.max(0, Math.min(selected, items.size() - 1));
        final NotificationState.Item item = items.get(selected);
        cardArea.addView(NotificationCards.bind(getPackageName(), item, selected + 1, items.size(), null).apply(this, cardArea));
        for (final NotificationState.Action a : item.actions) {
            if (a.reply) continue;   // dictated replies: plan 5b
            menuBar.addView(button(a.title, new Runnable() {
                @Override public void run() { NotificationCommands.action(notifications, item.key, a.index); }
            }));
        }
        menuBar.addView(button("Dismiss", new Runnable() {
            @Override public void run() { NotificationCommands.dismiss(notifications, item.key); }
        }));
        if (items.size() > 1) menuBar.addView(button("Next", new Runnable() {
            @Override public void run() { selected = (selected + 1) % items.size(); render(); }
        }));
    }

    private Button button(String label, final Runnable onClick) {
        Button b = new Button(this);
        b.setText(label);
        b.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) { onClick.run(); }
        });
        return b;
    }

    @Override
    protected void onDestroy() {
        if (server != null) server.close();
        super.onDestroy();
    }
}
