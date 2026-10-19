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
                ? null : decodeIcon(card.iconPng);
        if (icon != null) views.setImageViewBitmap(R.id.nav_icon, icon);
        else views.setImageViewResource(R.id.nav_icon, ManeuverArrows.drawableFor(card.maneuver));
        setText(views, R.id.nav_distance, formatDistance(card.distanceM));
        setText(views, R.id.nav_street, card.street);
        setText(views, R.id.nav_instruction, card.instruction);
        setText(views, R.id.nav_eta, card.eta == null ? null : "ETA " + card.eta);
        return views;
    }

    /** Icons forwarded by the phone are decoded at most ~{@link #ICON_MAX_PX} px: RemoteViews and memory stay small. */
    static final int ICON_MAX_PX = 160;

    static Bitmap decodeIcon(byte[] png) {
        BitmapFactory.Options bounds = new BitmapFactory.Options();
        bounds.inJustDecodeBounds = true;
        BitmapFactory.decodeByteArray(png, 0, png.length, bounds);
        int sample = sampleSizeFor(bounds.outWidth, bounds.outHeight, ICON_MAX_PX);
        if (sample == 0) return null;
        BitmapFactory.Options options = new BitmapFactory.Options();
        options.inSampleSize = sample;
        Bitmap decoded = BitmapFactory.decodeByteArray(png, 0, png.length, options);
        if (decoded == null) return null;
        int max = Math.max(decoded.getWidth(), decoded.getHeight());
        if (max <= ICON_MAX_PX) return decoded;
        float scale = ICON_MAX_PX / (float) max;
        return Bitmap.createScaledBitmap(decoded, Math.max(1, (int) (decoded.getWidth() * scale)),
                Math.max(1, (int) (decoded.getHeight() * scale)), true);
    }

    /** Power-of-two decode sample size so both sides stay at least {@code max}; 0 if the bounds are unreadable. */
    static int sampleSizeFor(int width, int height, int max) {
        if (width <= 0 || height <= 0) return 0;
        int sample = 1;
        while (width / (sample * 2) >= max && height / (sample * 2) >= max) sample *= 2;
        return sample;
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
