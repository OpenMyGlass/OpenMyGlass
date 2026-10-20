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
    /** How long a dismissed card stays hidden waiting for the phone's removal before it comes back. */
    static final long DISMISS_CONFIRM_MS = 5_000;

    private final List<Bridge.Notification> protos = new ArrayList<Bridge.Notification>();   // newest first
    /** Dismissed here, waiting for the phone to confirm the removal (the phone may refuse: FLAG_NO_CLEAR). */
    private final java.util.Map<String, org.openmyglass.glass.nav.NavCardState.Cancellable> hidden =
            new java.util.HashMap<String, org.openmyglass.glass.nav.NavCardState.Cancellable>();
    private final org.openmyglass.glass.nav.NavCardState.Scheduler scheduler;
    private BridgeServer.Sender sender;

    public NotificationState(IconCache icons, Listener listener, int cap) {
        this(icons, listener, cap, timerScheduler());
    }

    public NotificationState(IconCache icons, Listener listener, int cap, org.openmyglass.glass.nav.NavCardState.Scheduler scheduler) {
        this.icons = icons;
        this.listener = listener;
        this.cap = cap;
        this.scheduler = scheduler;
    }

    private static org.openmyglass.glass.nav.NavCardState.Scheduler timerScheduler() {
        final java.util.Timer timer = new java.util.Timer("notification-timers", true);
        return new org.openmyglass.glass.nav.NavCardState.Scheduler() {
            @Override public org.openmyglass.glass.nav.NavCardState.Cancellable schedule(final Runnable task, long delayMs) {
                final java.util.TimerTask t = new java.util.TimerTask() { @Override public void run() { task.run(); } };
                timer.schedule(t, delayMs);
                return new org.openmyglass.glass.nav.NavCardState.Cancellable() { @Override public void cancel() { t.cancel(); } };
            }
        };
    }

    @Override
    public synchronized void onSessionStarted(BridgeServer.Sender sender) {
        this.sender = sender;
        for (org.openmyglass.glass.nav.NavCardState.Cancellable c : hidden.values()) c.cancel();
        hidden.clear();
        if (!protos.isEmpty()) {
            protos.clear();
            changed();
        }
    }

    @Override
    public void onFrame(Bridge.Frame frame) {
        Bridge.Frame request;
        synchronized (this) {
            request = apply(frame);
        }
        if (request != null) sendQuietly(request);   // outside the lock: a stalled write must not block items()
    }

    /** Applies a frame; returns an icon request to send, if any. */
    private Bridge.Frame apply(Bridge.Frame frame) {
        Bridge.Frame request = null;
        if (frame.hasNotification()) {
            Bridge.Notification n = frame.getNotification().getNotification();
            int at = indexOf(n.getKey());
            if (at >= 0) {
                protos.set(at, n);
            } else {
                protos.add(0, n);
                while (protos.size() > cap) protos.remove(protos.size() - 1);
            }
            request = iconRequestIfMissing(n);
            changed();
            if (at < 0) listener.onNew(item(n), n.getWake());
        } else if (frame.hasNotificationRemoved()) {
            String key = frame.getNotificationRemoved().getKey();
            org.openmyglass.glass.nav.NavCardState.Cancellable restore = hidden.remove(key);
            if (restore != null) restore.cancel();
            int at = indexOf(key);
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
        return request;
    }

    @Override
    public synchronized void onSessionEnded(BridgeServer.Sender ended) {
        if (ended == sender) sender = null;
    }

    /**
     * Asks the phone to run one of the notification's actions; false if it is gone or there is no session.
     * Blocks on the bridge write: call off the main thread (NotificationCommands).
     */
    public boolean requestAction(String key, int index) {
        BridgeServer.Sender s;
        synchronized (this) {
            if (indexOf(key) < 0) return false;
            s = sender;
        }
        return send(s, Bridge.NotificationAction.newBuilder().setKey(key).setActionIndex(index).build());
    }

    /** Dismisses on the phone (and here right away); false if it is gone or there is no session. */
    public boolean dismiss(String key) {
        BridgeServer.Sender s;
        synchronized (this) {
            if (indexOf(key) < 0) return false;
            s = sender;
        }
        if (!send(s, Bridge.NotificationAction.newBuilder().setKey(key).setDismiss(true).build())) return false;
        synchronized (this) {
            // Hidden now, removed when the phone confirms; back if it does not (the phone kept it)
            if (indexOf(key) >= 0 && !hidden.containsKey(key)) {
                final String k = key;
                hidden.put(key, scheduler.schedule(new Runnable() { @Override public void run() {
                    synchronized (NotificationState.this) {
                        if (hidden.remove(k) != null) changed();
                    }
                } }, DISMISS_CONFIRM_MS));
                changed();
            }
        }
        return true;
    }

    public synchronized List<Item> items() {
        List<Item> list = new ArrayList<Item>(protos.size());
        for (Bridge.Notification n : protos) if (!hidden.containsKey(n.getKey())) list.add(item(n));
        return list;
    }

    private static boolean send(BridgeServer.Sender s, Bridge.NotificationAction action) {
        if (s == null) return false;
        try {
            s.send(Bridge.Frame.newBuilder().setNotificationAction(action).build());
            return true;
        } catch (IOException e) {
            return false;
        }
    }

    private void sendQuietly(Bridge.Frame frame) {
        BridgeServer.Sender s;
        synchronized (this) {
            s = sender;
        }
        if (s == null) return;
        try {
            s.send(frame);
        } catch (IOException e) {
            // the session is ending; the next one resends everything
        }
    }

    private Bridge.Frame iconRequestIfMissing(Bridge.Notification n) {
        if (!n.hasIconSha256() || sender == null) return null;
        String hex = hex(n.getIconSha256().toByteArray());
        if (icons.get(hex) != null || !icons.markRequested(hex)) return null;
        return Bridge.Frame.newBuilder().setIconRequest(Bridge.IconRequest.newBuilder().setSha256(n.getIconSha256())).build();
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
