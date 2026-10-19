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

import org.openmyglass.glass.bridge.BridgeServer;
import org.openmyglass.glass.bridge.ServerChannel;
import org.openmyglass.glass.bridge.TcpServerChannel;
import org.openmyglass.glass.nav.CardBinder;
import org.openmyglass.glass.nav.HandlerScheduler;
import org.openmyglass.glass.nav.IconCache;
import org.openmyglass.glass.nav.NavCardState;
import org.openmyglass.glass.nav.WakePolicy;

/**
 * Runs the bridge server and shows navigation as a GDK LiveCard (RemoteViews: low battery cost, updated only
 * when the state changes). The screen is woken only per {@link WakePolicy}. The phone connects over
 * Bluetooth RFCOMM, or over TCP on {@link #PORT} for debugging.
 */
public final class NavLiveCardService extends Service {
    public static final int PORT = 7825;
    private static final long WAKE_MS = 5_000;

    private final Handler main = new Handler(Looper.getMainLooper());
    private BridgeServer server;
    private LiveCard liveCard;
    private NavCardState.Card shown;
    private PowerManager.WakeLock wakeLock;

    @Override
    public void onCreate() {
        super.onCreate();
        PowerManager pm = (PowerManager) getSystemService(Context.POWER_SERVICE);
        @SuppressWarnings("deprecation")
        int flags = PowerManager.SCREEN_BRIGHT_WAKE_LOCK | PowerManager.ACQUIRE_CAUSES_WAKEUP | PowerManager.ON_AFTER_RELEASE;
        wakeLock = pm.newWakeLock(flags, "openmyglass:nav");
        wakeLock.setReferenceCounted(false);
        NavCardState state = new NavCardState(new IconCache(64), new NavCardState.Listener() {
            @Override public void onChanged(final NavCardState.Card card) {
                main.post(new Runnable() { @Override public void run() { show(card); } });
            }
            @Override public void onEnded() {
                main.post(new Runnable() { @Override public void run() { end(); } });
            }
        }, new HandlerScheduler(main), 30_000);
        // RFCOMM for the phone (bonding authenticates it); TCP stays as the documented debug path
        server = new BridgeServer(java.util.Arrays.<ServerChannel>asList(new RfcommServerChannel(), new TcpServerChannel(PORT)),
                state, BridgeServer.DEFAULT_HELLO_TIMEOUT_MS);
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
        if (server != null) server.close();
        super.onDestroy();
    }
}
