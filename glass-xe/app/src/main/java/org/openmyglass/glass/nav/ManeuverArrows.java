package org.openmyglass.glass.nav;

import org.openmyglass.glass.R;
import org.openmyglass.protocol.v1.Bridge;

/** Maneuver → OpenMyGlass arrow drawable (original vector artwork, rasterized at build time for API 19). */
public final class ManeuverArrows {
    private ManeuverArrows() {}

    public static int drawableFor(Bridge.Maneuver maneuver) {
        switch (maneuver) {
            case STRAIGHT: return R.drawable.nav_straight;
            case LEFT: return R.drawable.nav_left;
            case SLIGHT_LEFT: return R.drawable.nav_slight_left;
            case SHARP_LEFT: return R.drawable.nav_sharp_left;
            case RIGHT: return R.drawable.nav_right;
            case SLIGHT_RIGHT: return R.drawable.nav_slight_right;
            case SHARP_RIGHT: return R.drawable.nav_sharp_right;
            case KEEP_LEFT: return R.drawable.nav_keep_left;
            case KEEP_RIGHT: return R.drawable.nav_keep_right;
            case U_TURN_LEFT: return R.drawable.nav_uturn_left;
            case U_TURN_RIGHT: return R.drawable.nav_uturn_right;
            case OFF_ROUTE: return R.drawable.nav_off_route;
            case ROUNDABOUT: return R.drawable.nav_roundabout;
            case ROUNDABOUT_LEFT: return R.drawable.nav_roundabout_left;
            case ARRIVE: return R.drawable.nav_arrive;
            default: return R.drawable.nav_unknown;
        }
    }
}
