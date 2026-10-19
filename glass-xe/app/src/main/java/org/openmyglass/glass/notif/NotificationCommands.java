package org.openmyglass.glass.notif;

import java.util.concurrent.Executor;
import java.util.concurrent.Executors;

/** Runs actions chosen on Glass off the main thread (they write to the bridge socket). */
public final class NotificationCommands {
    private static final Executor IO = Executors.newSingleThreadExecutor();

    private NotificationCommands() {}

    public static void action(final NotificationState state, final String key, final int index) {
        IO.execute(new Runnable() { @Override public void run() { state.requestAction(key, index); } });
    }

    public static void dismiss(final NotificationState state, final String key) {
        IO.execute(new Runnable() { @Override public void run() { state.dismiss(key); } });
    }
}
