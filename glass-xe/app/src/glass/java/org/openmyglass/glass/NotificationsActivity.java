package org.openmyglass.glass;

import android.app.Activity;
import android.os.Bundle;
import android.view.KeyEvent;
import android.view.Menu;
import android.view.MenuItem;
import android.widget.FrameLayout;

import java.util.List;

import org.openmyglass.glass.notif.NotificationCards;
import org.openmyglass.glass.notif.NotificationCommands;
import org.openmyglass.glass.notif.NotificationState;

/**
 * Immersion with one card per notification, newest first. Swipe (D-pad left/right, as the touchpad reports it
 * to plain activities) moves between cards; tap opens the menu with the notification's own actions and Dismiss.
 * Replies (actions with a RemoteInput) are not offered until dictation lands (plan 5b).
 */
public final class NotificationsActivity extends Activity {
    private static final int DISMISS = -1;

    private FrameLayout frame;
    private int position;
    private final Runnable refresh = new Runnable() {
        @Override public void run() { render(); }
    };

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        frame = new FrameLayout(this);
        setContentView(frame);
    }

    @Override
    protected void onResume() {
        super.onResume();
        BridgeService.addNotificationListener(refresh);
        render();
    }

    @Override
    protected void onPause() {
        BridgeService.removeNotificationListener(refresh);
        super.onPause();
    }

    private List<NotificationState.Item> items() {
        NotificationState state = BridgeService.notifications();
        return state == null ? java.util.Collections.<NotificationState.Item>emptyList() : state.items();
    }

    private void render() {
        List<NotificationState.Item> items = items();
        if (items.isEmpty()) {
            finish();
            return;
        }
        position = Math.max(0, Math.min(position, items.size() - 1));
        frame.removeAllViews();
        frame.addView(NotificationCards.bind(getPackageName(), items.get(position), position + 1, items.size(), "tap for actions")
                .apply(this, frame));
    }

    @Override
    public boolean onKeyDown(int keyCode, KeyEvent event) {
        boolean back = event.isShiftPressed();
        switch (keyCode) {
            case KeyEvent.KEYCODE_DPAD_RIGHT:
                position++;
                render();
                return true;
            case KeyEvent.KEYCODE_DPAD_LEFT:
                position--;
                render();
                return true;
            case KeyEvent.KEYCODE_TAB:
                position += back ? -1 : 1;
                render();
                return true;
            case KeyEvent.KEYCODE_DPAD_CENTER:
            case KeyEvent.KEYCODE_ENTER:
                openOptionsMenu();
                return true;
            default:
                return super.onKeyDown(keyCode, event);
        }
    }

    @Override
    public boolean onCreateOptionsMenu(Menu menu) {
        return true;
    }

    @Override
    public boolean onPrepareOptionsMenu(Menu menu) {
        menu.clear();
        List<NotificationState.Item> items = items();
        if (items.isEmpty()) return false;
        NotificationState.Item item = items.get(Math.min(position, items.size() - 1));
        int order = 0;
        for (NotificationState.Action a : item.actions) {
            if (!a.reply) menu.add(Menu.NONE, a.index, order++, a.title);
        }
        menu.add(Menu.NONE, DISMISS, order, "Dismiss");
        return true;
    }

    @Override
    public boolean onOptionsItemSelected(MenuItem menuItem) {
        NotificationState state = BridgeService.notifications();
        List<NotificationState.Item> items = items();
        if (state == null || items.isEmpty()) return true;
        String key = items.get(Math.min(position, items.size() - 1)).key;
        if (menuItem.getItemId() == DISMISS) NotificationCommands.dismiss(state, key);
        else NotificationCommands.action(state, key, menuItem.getItemId());
        return true;
    }
}
