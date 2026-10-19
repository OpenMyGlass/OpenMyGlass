package org.openmyglass.glass;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;

/** Launcher entry: starts the bridge/LiveCard service and closes. Boot start and voice trigger come later. */
public final class StartActivity extends Activity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        startService(new Intent(this, NavLiveCardService.class));
        finish();
    }
}
