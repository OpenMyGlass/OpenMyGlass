package org.openmyglass.glass.notif;

import java.io.IOException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

import org.openmyglass.glass.bridge.BridgeServer;
import org.openmyglass.glass.nav.IconCache;
import org.openmyglass.protocol.v1.Bridge;

/**
 * The phone notifications shown on Glass, newest first and capped. Each bridge session starts empty: the phone
 * resends its whole current set, so stale notifications never survive a reconnect. Frames arrive on the session
 * thread; listener callbacks run there too (renderers post to the main thread).
 */
public final class NotificationState implements BridgeServer.FrameHandler {

    public interface Listener {
        void onChanged(List<Item> items);
        /** A notification that was not shown before; {@code wake} if the owner allowed it to wake the screen. */
        void onNew(Item item, boolean wake);
    }

    public static final class Action {
        public final int index;
        public final String title;
        public final boolean reply;

        Action(Bridge.NotificationActionInfo a) {
            index = a.getIndex();
            title = a.getTitle();
            reply = a.getReply();
        }
    }

    public static final class Item {
        public final String key;
        public final String appName;
        public final String title;
        public final String text;
        public final long postedAtMs;
        public final String iconHex;
        public final byte[] iconPng;
        public final List<Action> actions;

        Item(Bridge.Notification n, String iconHex, byte[] iconPng) {
            key = n.getKey();
            appName = n.getAppName();
            title = n.getTitle();
            text = n.getText();
            postedAtMs = n.getPostedAtMs();
            this.iconHex = iconHex;
            this.iconPng = iconPng;
            List<Action> list = new ArrayList<Action>();
            for (Bridge.NotificationActionInfo a : n.getActionsList()) list.add(new Action(a));
            actions = Collections.unmodifiableList(list);
        }
    }

    private final IconCache icons;
    private final Listener listener;
    private final int cap;
    private final List<Bridge.Notification> protos = new ArrayList<Bridge.Notification>();   // newest first
    private BridgeServer.Sender sender;

    public NotificationState(IconCache icons, Listener listener, int cap) {
        this.icons = icons;
        this.listener = listener;
        this.cap = cap;
    }

    @Override
    public synchronized void onSessionStarted(BridgeServer.Sender sender) {
        this.sender = sender;
        if (!protos.isEmpty()) {
            protos.clear();
            changed();
        }
    }

    @Override
    public synchronized void onFrame(Bridge.Frame frame) {
        if (frame.hasNotification()) {
            Bridge.Notification n = frame.getNotification().getNotification();
            int at = indexOf(n.getKey());
            if (at >= 0) {
                protos.set(at, n);
            } else {
                protos.add(0, n);
                while (protos.size() > cap) protos.remove(protos.size() - 1);
            }
            requestIconIfMissing(n);
            changed();
            if (at < 0) listener.onNew(item(n), n.getWake());
        } else if (frame.hasNotificationRemoved()) {
            int at = indexOf(frame.getNotificationRemoved().getKey());
            if (at >= 0) {
                protos.remove(at);
                changed();
            }
        } else if (frame.hasIcon()) {
            String hex = hex(frame.getIcon().getSha256().toByteArray());
            icons.put(hex, frame.getIcon().getPng().toByteArray());
            for (Bridge.Notification n : protos) {
                if (n.hasIconSha256() && hex.equals(hex(n.getIconSha256().toByteArray()))) {
                    changed();
                    break;
                }
            }
        }
    }

    @Override
    public synchronized void onSessionEnded(BridgeServer.Sender ended) {
        if (ended == sender) sender = null;
    }

    /** Asks the phone to run one of the notification's actions; false if it is gone or there is no session. */
    public synchronized boolean requestAction(String key, int index) {
        if (indexOf(key) < 0) return false;
        return send(Bridge.NotificationAction.newBuilder().setKey(key).setActionIndex(index).build());
    }

    /** Dismisses on the phone (and here right away); false if it is gone or there is no session. */
    public synchronized boolean dismiss(String key) {
        int at = indexOf(key);
        if (at < 0) return false;
        if (!send(Bridge.NotificationAction.newBuilder().setKey(key).setDismiss(true).build())) return false;
        protos.remove(at);
        changed();
        return true;
    }

    public synchronized List<Item> items() {
        List<Item> list = new ArrayList<Item>(protos.size());
        for (Bridge.Notification n : protos) list.add(item(n));
        return list;
    }

    private boolean send(Bridge.NotificationAction action) {
        if (sender == null) return false;
        try {
            sender.send(Bridge.Frame.newBuilder().setNotificationAction(action).build());
            return true;
        } catch (IOException e) {
            return false;
        }
    }

    private void requestIconIfMissing(Bridge.Notification n) {
        if (!n.hasIconSha256() || sender == null) return;
        String hex = hex(n.getIconSha256().toByteArray());
        if (icons.get(hex) != null || !icons.markRequested(hex)) return;
        try {
            sender.send(Bridge.Frame.newBuilder().setIconRequest(Bridge.IconRequest.newBuilder().setSha256(n.getIconSha256())).build());
        } catch (IOException e) {
            // the session is ending; the next one resends everything
        }
    }

    private void changed() {
        listener.onChanged(items());
    }

    private Item item(Bridge.Notification n) {
        String hex = n.hasIconSha256() ? hex(n.getIconSha256().toByteArray()) : null;
        return new Item(n, hex, hex == null ? null : icons.get(hex));
    }

    private int indexOf(String key) {
        for (int i = 0; i < protos.size(); i++) if (protos.get(i).getKey().equals(key)) return i;
        return -1;
    }

    private static String hex(byte[] bytes) {
        StringBuilder sb = new StringBuilder(bytes.length * 2);
        for (byte b : bytes) sb.append(String.format("%02x", b));
        return sb.toString();
    }
}
