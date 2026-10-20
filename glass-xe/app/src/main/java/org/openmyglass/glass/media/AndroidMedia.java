package org.openmyglass.glass.media;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.net.ConnectivityManager;
import android.net.NetworkInfo;
import android.net.wifi.WifiManager;
import android.os.BatteryManager;
import android.os.FileObserver;

import java.io.File;
import java.util.Locale;
import java.util.concurrent.Executors;

/**
 * Android glue for media sync: charging and Wi-Fi from system broadcasts and new files from a FileObserver
 * (event-driven, no polling, architecture §6). Handler calls write to the bridge, so they run on a worker thread.
 */
public final class AndroidMedia implements MediaHandler.Status {
    private final Context context;
    private final String ipOverride;
    private final java.util.concurrent.ScheduledExecutorService worker = Executors.newSingleThreadScheduledExecutor();
    private MediaHandler handler;
    private FileObserver observer;
    private Boolean lastCharging;
    private Boolean lastWifi;
    private volatile boolean wifiForcedOff;

    /** Debug (emulator): pretend Glass has no Wi-Fi, to exercise the Bluetooth fallback. */
    public void setWifiForcedOff(boolean off) {
        wifiForcedOff = off;
        final MediaHandler h = handler;
        if (h != null) worker.execute(new Runnable() { @Override public void run() { h.onStatusChanged(); } });
    }

    private final BroadcastReceiver changes = new BroadcastReceiver() {
        @Override public void onReceive(Context c, Intent intent) {
            boolean charging = charging();
            boolean wifi = wifi();
            if (Boolean.valueOf(charging).equals(lastCharging) && Boolean.valueOf(wifi).equals(lastWifi)) return;
            lastCharging = charging;
            lastWifi = wifi;
            final MediaHandler h = handler;
            if (h != null) worker.execute(new Runnable() { @Override public void run() { h.onStatusChanged(); } });
        }
    };

    /** @param ipOverride address to announce instead of the Wi-Fi one (emulator: phone and Glass on one device) */
    public AndroidMedia(Context context, String ipOverride) {
        this.context = context.getApplicationContext();
        this.ipOverride = ipOverride;
    }

    private long settleMs = MediaIndex.DEFAULT_SETTLE_MS;

    public void start(MediaHandler handler, File dir, long settleMs) {
        this.settleMs = settleMs;
        start(handler, dir);
    }

    public void start(MediaHandler handler, File dir) {
        this.handler = handler;
        IntentFilter filter = new IntentFilter();
        filter.addAction(Intent.ACTION_POWER_CONNECTED);
        filter.addAction(Intent.ACTION_POWER_DISCONNECTED);
        filter.addAction(Intent.ACTION_BATTERY_CHANGED);   // sticky; only plug changes are forwarded
        filter.addAction(ConnectivityManager.CONNECTIVITY_ACTION);
        context.registerReceiver(changes, filter);
        dir.mkdirs();
        final MediaHandler h = handler;
        observer = new FileObserver(dir.getAbsolutePath(), FileObserver.CLOSE_WRITE | FileObserver.MOVED_TO) {
            @Override public void onEvent(int event, String path) {
                // The index leaves out files younger than its settle time: announce once this one has settled
                worker.schedule(new Runnable() { @Override public void run() { h.onNewMedia(); } },
                        settleMs + 1_000, java.util.concurrent.TimeUnit.MILLISECONDS);
            }
        };
        observer.startWatching();
    }

    public void stop() {
        if (observer != null) observer.stopWatching();
        try {
            context.unregisterReceiver(changes);
        } catch (IllegalArgumentException ignored) {
            // not registered
        }
        handler = null;
        worker.shutdown();
    }

    @Override
    public boolean charging() {
        Intent battery = context.registerReceiver(null, new IntentFilter(Intent.ACTION_BATTERY_CHANGED));
        return battery != null && battery.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) != 0;
    }

    @Override
    @SuppressWarnings("deprecation")
    public boolean wifi() {
        if (wifiForcedOff) return false;
        ConnectivityManager cm = (ConnectivityManager) context.getSystemService(Context.CONNECTIVITY_SERVICE);
        NetworkInfo info = cm == null ? null : cm.getActiveNetworkInfo();
        return info != null && info.isConnected() && info.getType() == ConnectivityManager.TYPE_WIFI;
    }

    @Override
    @SuppressWarnings("deprecation")
    public String ip() {
        if (!wifi()) return "";
        if (ipOverride != null) return ipOverride;
        WifiManager wm = (WifiManager) context.getApplicationContext().getSystemService(Context.WIFI_SERVICE);
        int ip = wm == null ? 0 : wm.getConnectionInfo().getIpAddress();
        if (ip == 0) return "";
        return String.format(Locale.ROOT, "%d.%d.%d.%d", ip & 0xff, (ip >> 8) & 0xff, (ip >> 16) & 0xff, (ip >> 24) & 0xff);
    }
}
