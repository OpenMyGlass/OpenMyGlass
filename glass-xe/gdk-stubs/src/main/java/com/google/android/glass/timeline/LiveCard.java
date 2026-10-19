package com.google.android.glass.timeline;

import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.widget.RemoteViews;

/**
 * Compile-only stub, written by OpenMyGlass from the public GDK reference:
 * https://developers.google.com/glass/develop/gdk/reference/com/google/android/glass/timeline/LiveCard
 * (documentation content CC BY 4.0). Only the members OpenMyGlass uses are declared.
 * The real implementation is provided by the Glass system at runtime.
 */
public class LiveCard {

    /** How a card is published: REVEAL shows it immediately, SILENT adds it to the timeline quietly. */
    public enum PublishMode { REVEAL, SILENT }

    public LiveCard(Context context, String tag) {
        throw new RuntimeException("Stub!");
    }

    public LiveCard setViews(RemoteViews views) {
        throw new RuntimeException("Stub!");
    }

    public LiveCard setAction(PendingIntent intent) {
        throw new RuntimeException("Stub!");
    }

    public LiveCard attach(Service service) {
        throw new RuntimeException("Stub!");
    }

    public void publish(PublishMode mode) {
        throw new RuntimeException("Stub!");
    }

    public void unpublish() {
        throw new RuntimeException("Stub!");
    }

    public boolean isPublished() {
        throw new RuntimeException("Stub!");
    }

    public void navigate() {
        throw new RuntimeException("Stub!");
    }
}
