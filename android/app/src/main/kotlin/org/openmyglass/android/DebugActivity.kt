package org.openmyglass.android

import android.app.Activity
import android.os.Bundle
import android.widget.TextView

/** Developer screen: connection and navigation-source status. Grows as features land. */
class DebugActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(TextView(this).apply { text = "OpenMyGlass" })
    }
}
