package org.openmyglass.glass.nav;

/**
 * When to turn the Glass screen on for the navigation card (battery rules, architecture §6): on the first card,
 * when the maneuver changes, and when the distance crosses below {@link #NEAR_METERS}. Never on small distance
 * changes, never when navigation ends.
 */
public final class WakePolicy {
    public static final int NEAR_METERS = 200;

    private WakePolicy() {}

    public static boolean shouldWake(NavCardState.Card previous, NavCardState.Card next) {
        if (next == null) return false;
        if (previous == null) return true;
        if (previous.maneuver != next.maneuver) return true;
        boolean wasFar = previous.distanceM == null || previous.distanceM >= NEAR_METERS;
        boolean isNear = next.distanceM != null && next.distanceM < NEAR_METERS;
        return wasFar && isNear;
    }
}
