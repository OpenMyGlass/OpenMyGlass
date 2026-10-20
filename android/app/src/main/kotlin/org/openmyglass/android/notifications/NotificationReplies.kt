package org.openmyglass.android.notifications

import android.app.ActivityOptions
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import org.openmyglass.core.dictation.ReplyOutcome

/** Sends a dictated reply through the notification's RemoteInput action. */
object NotificationReplies {
    fun reply(context: Context, key: String, actionIndex: Int, text: String): ReplyOutcome {
        val listener = OpenMyGlassNotificationListener.instance ?: return ReplyOutcome(false, "Notification access is off")
        val sbn = runCatching { listener.getActiveNotifications(arrayOf(key)) }.getOrNull()?.firstOrNull()
            ?: return ReplyOutcome(false, "The notification is gone")
        val action = sbn.notification.actions?.getOrNull(actionIndex) ?: return ReplyOutcome(false, "The reply action is gone")
        val inputs = action.remoteInputs?.takeIf { it.isNotEmpty() } ?: return ReplyOutcome(false, "Not a reply action")
        val results = Bundle().apply { inputs.forEach { putCharSequence(it.resultKey, text) } }
        val intent = Intent()
        RemoteInput.addResultsToIntent(inputs, intent, results)
        return try {
            if (Build.VERSION.SDK_INT >= 34) {
                val options = ActivityOptions.makeBasic()
                    .setPendingIntentBackgroundActivityStartMode(ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED)
                action.actionIntent.send(context, 0, intent, null, null, null, options.toBundle())
            } else {
                action.actionIntent.send(context, 0, intent)
            }
            ReplyOutcome(true, "Sent")
        } catch (e: PendingIntent.CanceledException) {
            ReplyOutcome(false, "The app refused the reply")
        }
    }
}
