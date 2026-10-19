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
import org.openmyglass.glass.notif.NotificationCursor;
import org.openmyglass.glass.notif.NotificationState;

/**
 * Immersion with one card per notification, newest first. Swipe (D-pad left/right, as the touchpad reports it
 * to plain activities) moves between cards; tap opens the menu with the notification's own actions and Dismiss.
 * Replies (actions with a RemoteInput) are not offered until dictation lands (plan 5b).
 */
public final class NotificationsActivity extends Activity {
    private static final int DISMISS = -1;

    private FrameLayout frame;
    /** Selection by key: arrivals and removals never move it to another notification. */
    private final NotificationCursor cursor = new NotificationCursor();
    /** The notification the open menu was built for: its items act on this key only. */
    private String menuKey;
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

    private static List<String> keys(List<NotificationState.Item> items) {
        List<String> keys = new java.util.ArrayList<String>(items.size());
        for (NotificationState.Item i : items) keys.add(i.key);
        return keys;
    }

    private NotificationState.Item find(String key) {
        for (NotificationState.Item i : items()) if (i.key.equals(key)) return i;
        return null;
    }

    private void render() {
        List<NotificationState.Item> items = items();
        int at = cursor.index(keys(items));
        if (at < 0) {
            finish();
            return;
        }
        frame.removeAllViews();
        frame.addView(NotificationCards.bind(getPackageName(), items.get(at), at + 1, items.size(), "tap for actions")
                .apply(this, frame));
    }

    @Override
    public boolean onKeyDown(int keyCode, KeyEvent event) {
        boolean back = event.isShiftPressed();
        List<String> keys = keys(items());
        switch (keyCode) {
            case KeyEvent.KEYCODE_DPAD_RIGHT:
                cursor.next(keys);
                render();
                return true;
            case KeyEvent.KEYCODE_DPAD_LEFT:
                cursor.previous(keys);
                render();
                return true;
            case KeyEvent.KEYCODE_TAB:
                if (back) cursor.previous(keys); else cursor.next(keys);
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
        cursor.index(keys(items()));
        NotificationState.Item item = cursor.key() == null ? null : find(cursor.key());
        if (item == null) return false;
        menuKey = item.key;
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
        if (state == null || menuKey == null) return true;
        // Acts on the notification the menu was built for; if it is gone meanwhile, the state ignores it
        if (menuItem.getItemId() == DISMISS) NotificationCommands.dismiss(state, menuKey);
        else NotificationCommands.action(state, menuKey, menuItem.getItemId());
        return true;
    }
}
