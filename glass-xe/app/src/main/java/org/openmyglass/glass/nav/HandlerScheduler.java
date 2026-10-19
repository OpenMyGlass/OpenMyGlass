package org.openmyglass.glass.nav;

import android.os.Handler;

/** {@link NavCardState.Scheduler} on an Android Handler. */
public final class HandlerScheduler implements NavCardState.Scheduler {
    private final Handler handler;

    public HandlerScheduler(Handler handler) {
        this.handler = handler;
    }

    @Override
    public NavCardState.Cancellable schedule(final Runnable task, long delayMs) {
        handler.postDelayed(task, delayMs);
        return new NavCardState.Cancellable() {
            @Override public void cancel() { handler.removeCallbacks(task); }
        };
    }
}
