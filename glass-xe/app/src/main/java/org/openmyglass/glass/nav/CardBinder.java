package org.openmyglass.glass.nav;

import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.view.View;
import android.widget.RemoteViews;

import org.openmyglass.glass.R;

/** Fills the nav_card layout from a {@link NavCardState.Card}; shared by the LiveCard and the emulator renderer. */
public final class CardBinder {
    private CardBinder() {}

    public static RemoteViews bind(String packageName, NavCardState.Card card) {
        RemoteViews views = new RemoteViews(packageName, R.layout.nav_card);
        Bitmap icon = card.iconPng == null || card.maneuver != org.openmyglass.protocol.v1.Bridge.Maneuver.MANEUVER_UNKNOWN
                ? null : BitmapFactory.decodeByteArray(card.iconPng, 0, card.iconPng.length);
        if (icon != null) views.setImageViewBitmap(R.id.nav_icon, icon);
        else views.setImageViewResource(R.id.nav_icon, ManeuverArrows.drawableFor(card.maneuver));
        setText(views, R.id.nav_distance, formatDistance(card.distanceM));
        setText(views, R.id.nav_street, card.street);
        setText(views, R.id.nav_instruction, card.instruction);
        setText(views, R.id.nav_eta, card.eta == null ? null : "ETA " + card.eta);
        return views;
    }

    /** "850 m", "1.2 km"; null when unknown. */
    public static String formatDistance(Integer meters) {
        if (meters == null) return null;
        if (meters < 1000) return meters + " m";
        return String.format(java.util.Locale.ROOT, "%.1f km", meters / 1000.0);
    }

    private static void setText(RemoteViews views, int id, String text) {
        views.setTextViewText(id, text == null ? "" : text);
        views.setViewVisibility(id, text == null || text.isEmpty() ? View.GONE : View.VISIBLE);
    }
}
