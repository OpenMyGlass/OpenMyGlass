package org.openmyglass.glass.notif;

import java.util.List;

/**
 * Which notification card is selected, by key: notifications arriving or leaving do not move the selection to
 * another notification. If the selected one is removed, the card now at its position is selected.
 */
public final class NotificationCursor {
    private String key;
    private int lastIndex;

    /** The selected card's index in {@code keys} (newest first), or -1 if there are none. */
    public int index(List<String> keys) {
        if (keys.isEmpty()) {
            key = null;
            return -1;
        }
        int at = key == null ? -1 : keys.indexOf(key);
        if (at < 0) at = Math.max(0, Math.min(lastIndex, keys.size() - 1));
        key = keys.get(at);
        lastIndex = at;
        return at;
    }

    public void next(List<String> keys) {
        move(keys, 1);
    }

    public void previous(List<String> keys) {
        move(keys, -1);
    }

    public String key() {
        return key;
    }

    private void move(List<String> keys, int delta) {
        int at = index(keys);
        if (at < 0) return;
        at = Math.max(0, Math.min(at + delta, keys.size() - 1));
        key = keys.get(at);
        lastIndex = at;
    }
}
