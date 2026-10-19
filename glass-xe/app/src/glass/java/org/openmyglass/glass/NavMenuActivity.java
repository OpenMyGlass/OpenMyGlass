package org.openmyglass.glass;

import android.app.Activity;
import android.os.Bundle;

/** LiveCards require an action; a menu (stop, settings) comes later. For now tapping the card just returns. */
public final class NavMenuActivity extends Activity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        finish();
    }
}
