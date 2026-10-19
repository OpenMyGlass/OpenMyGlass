package org.openmyglass.glass;

import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.PowerManager;

import com.google.android.glass.timeline.LiveCard;

import java.util.Arrays;
import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;

import org.openmyglass.glass.bridge.BridgeServer;
import org.openmyglass.glass.bridge.BridgeChannels;
import org.openmyglass.glass.bridge.FanOutHandler;
import org.openmyglass.glass.notif.NotificationCards;
import org.openmyglass.glass.notif.NotificationState;
import org.openmyglass.glass.nav.CardBinder;
import org.openmyglass.glass.nav.HandlerScheduler;
import org.openmyglass.glass.nav.IconCache;
import org.openmyglass.glass.nav.NavCardState;
import org.openmyglass.glass.nav.WakePolicy;

/**
 * Runs the bridge server and shows its features as GDK LiveCards (RemoteViews: low battery cost, updated only
 * when the state changes): the navigation card, and a notification card with the newest notification (tap opens
 * {@link NotificationsActivity}). The screen is woken only per {@link WakePolicy} and for new notifications the
 * owner allowed. The phone connects over Bluetooth RFCOMM, or over TCP on {@link #PORT} for debugging.
 */
public final class BridgeService extends Service {
    public static final int PORT = 7825;
    private static final long WAKE_MS = 5_000;

    private final Handler main = new Handler(Looper.getMainLooper());
    private BridgeServer server;
    private LiveCard liveCard;
    private NavCardState.Card shown;
    private PowerManager.WakeLock wakeLock;
    private LiveCard notificationCard;

    /** For NotificationsActivity: the running service's notifications, and change callbacks on the main thread. */
    private static volatile NotificationState notifications;
    private static final List<Runnable> notificationListeners = new CopyOnWriteArrayList<Runnable>();

    static NotificationState notifications() {
        return notifications;
    }

    static void addNotificationListener(Runnable r) {
        notificationListeners.add(r);
    }

    static void removeNotificationListener(Runnable r) {
        notificationListeners.remove(r);
    }

    @Override
    public void onCreate() {
        super.onCreate();
        PowerManager pm = (PowerManager) getSystemService(Context.POWER_SERVICE);
        @SuppressWarnings("deprecation")
        int flags = PowerManager.SCREEN_BRIGHT_WAKE_LOCK | PowerManager.ACQUIRE_CAUSES_WAKEUP | PowerManager.ON_AFTER_RELEASE;
        wakeLock = pm.newWakeLock(flags, "openmyglass:nav");
        wakeLock.setReferenceCounted(false);
        IconCache icons = new IconCache(64);
        NavCardState state = new NavCardState(icons, new NavCardState.Listener() {
            @Override public void onChanged(final NavCardState.Card card) {
                main.post(new Runnable() { @Override public void run() { show(card); } });
            }
            @Override public void onEnded() {
                main.post(new Runnable() { @Override public void run() { end(); } });
            }
        }, new HandlerScheduler(main), 30_000);
        notifications = new NotificationState(icons, new NotificationState.Listener() {
            @Override public void onChanged(final List<NotificationState.Item> items) {
                main.post(new Runnable() { @Override public void run() { showNotifications(items); } });
            }
            @Override public void onNew(NotificationState.Item item, final boolean wake) {
                if (wake) main.post(new Runnable() { @Override public void run() { wakeForNotification(); } });
            }
        }, 20);
        // RFCOMM for the phone (bonding authenticates it); the unauthenticated TCP debug port only on debuggable builds
        boolean debuggable = (getApplicationInfo().flags & android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0;
        server = new BridgeServer(BridgeChannels.forBuild(new RfcommServerChannel(), PORT, debuggable),
                new FanOutHandler(Arrays.<BridgeServer.FrameHandler>asList(state, notifications)),
                BridgeServer.DEFAULT_HELLO_TIMEOUT_MS);
        server.start();
    }

    private void show(NavCardState.Card card) {
        boolean wake = WakePolicy.shouldWake(shown, card);
        shown = card;
        if (liveCard == null) {
            liveCard = new LiveCard(this, "openmyglass-nav");
            Intent menu = new Intent(this, NavMenuActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK);
            liveCard.setAction(PendingIntent.getActivity(this, 0, menu, 0));
            liveCard.setViews(CardBinder.bind(getPackageName(), card));
            liveCard.attach(this);
            liveCard.publish(LiveCard.PublishMode.REVEAL);
        } else {
            liveCard.setViews(CardBinder.bind(getPackageName(), card));
        }
        if (wake) {
            liveCard.navigate();
            wakeLock.acquire(WAKE_MS);
        }
    }

    /** The notification LiveCard shows the newest notification; it is unpublished when there are none. */
    private void showNotifications(List<NotificationState.Item> items) {
        for (Runnable r : notificationListeners) r.run();
        if (items.isEmpty()) {
            if (notificationCard != null && notificationCard.isPublished()) notificationCard.unpublish();
            notificationCard = null;
            return;
        }
        if (notificationCard == null) {
            notificationCard = new LiveCard(this, "openmyglass-notifications");
            Intent open = new Intent(this, NotificationsActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK);
            notificationCard.setAction(PendingIntent.getActivity(this, 1, open, 0));
            notificationCard.setViews(NotificationCards.bind(getPackageName(), items.get(0), 1, items.size(), "tap to open"));
            notificationCard.attach(this);
            notificationCard.publish(LiveCard.PublishMode.SILENT);
        } else {
            notificationCard.setViews(NotificationCards.bind(getPackageName(), items.get(0), 1, items.size(), "tap to open"));
        }
    }

    /** A new notification the owner allowed to wake: bring its card forward and light the screen briefly. */
    private void wakeForNotification() {
        if (notificationCard == null) return;
        notificationCard.navigate();
        wakeLock.acquire(WAKE_MS);
    }

    private void end() {
        shown = null;
        if (wakeLock.isHeld()) wakeLock.release();
        if (liveCard != null) {
            if (liveCard.isPublished()) liveCard.unpublish();
            liveCard = null;
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        return START_STICKY;
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    @Override
    public void onDestroy() {
        end();
        if (notificationCard != null && notificationCard.isPublished()) notificationCard.unpublish();
        notificationCard = null;
        notifications = null;
        if (server != null) server.close();
        super.onDestroy();
    }
}
