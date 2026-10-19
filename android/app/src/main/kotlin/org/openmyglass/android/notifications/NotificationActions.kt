package org.openmyglass.android.notifications

import android.app.ActivityOptions
import android.app.PendingIntent
import android.content.Context
import android.os.Build
import android.util.Log
import org.openmyglass.core.notif.NotificationRelay

/** Carries out what the owner chose on Glass, through the notification listener. Failures are logged, never thrown. */
object NotificationActions {
    private const val TAG = "NotificationActions"

    fun perform(context: Context, key: String, request: NotificationRelay.ActionRequest) {
        val listener = OpenMyGlassNotificationListener.instance ?: return
        when (request) {
            NotificationRelay.ActionRequest.Dismiss -> runCatching { listener.cancelNotification(key) }
                .onFailure { Log.w(TAG, "dismiss failed: $it") }
            is NotificationRelay.ActionRequest.Fire -> {
                val sbn = runCatching { listener.getActiveNotifications(arrayOf(key)) }.getOrNull()?.firstOrNull() ?: return
                val action = sbn.notification.actions?.getOrNull(request.index) ?: return
                // Replies need dictated text: plan 5b. Glass does not offer them yet.
                if (!action.remoteInputs.isNullOrEmpty()) return
                try {
                    if (Build.VERSION.SDK_INT >= 34) {
                        val options = ActivityOptions.makeBasic()
                            .setPendingIntentBackgroundActivityStartMode(ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED)
                        action.actionIntent.send(context, 0, null, null, null, null, options.toBundle())
                    } else {
                        action.actionIntent.send()
                    }
                } catch (e: PendingIntent.CanceledException) {
                    Log.w(TAG, "action cancelled by its app: $e")   // Review Focus 5
                } catch (e: Exception) {
                    Log.w(TAG, "action failed: $e")
                }
            }
        }
    }
}
