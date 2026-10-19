package org.openmyglass.glass.notif;

import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.text.format.DateFormat;
import android.widget.RemoteViews;

import java.util.Date;

import org.openmyglass.glass.R;

/** Fills the notification card (RemoteViews: used by the LiveCard, the immersion and the emulator). */
public final class NotificationCards {
    private NotificationCards() {}

    /**
     * @param position 1-based position of {@code item} among {@code count} notifications
     * @param footer   hint line under the text (e.g. "tap for actions"), or null
     */
    public static RemoteViews bind(String packageName, NotificationState.Item item, int position, int count, String footer) {
        RemoteViews views = new RemoteViews(packageName, R.layout.notif_card);
        Bitmap icon = item.iconPng == null ? null : BitmapFactory.decodeByteArray(item.iconPng, 0, item.iconPng.length);
        if (icon != null) views.setImageViewBitmap(R.id.notif_icon, icon);
        String when = item.postedAtMs > 0 ? " · " + DateFormat.format("HH:mm", new Date(item.postedAtMs)) : "";
        views.setTextViewText(R.id.notif_app, item.appName + when);
        views.setTextViewText(R.id.notif_title, item.title);
        views.setTextViewText(R.id.notif_text, item.text);
        StringBuilder f = new StringBuilder();
        if (count > 1) f.append(position).append(" of ").append(count);
        if (footer != null) f.append(f.length() > 0 ? " · " : "").append(footer);
        views.setTextViewText(R.id.notif_footer, f.toString());
        return views;
    }
}
