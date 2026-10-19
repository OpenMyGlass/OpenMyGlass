package org.openmyglass.android.speech

import android.content.Context

/** Language of the offline speech model used for dictated replies. */
class SpeechSettings(context: Context) {
    private val prefs = context.getSharedPreferences("speech", Context.MODE_PRIVATE)

    var language: String
        get() = prefs.getString("language", "it") ?: "it"
        set(value) = prefs.edit().putString("language", value).apply()
}
