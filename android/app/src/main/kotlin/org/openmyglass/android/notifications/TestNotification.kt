package org.openmyglass.android.notifications

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import android.widget.Toast
import org.openmyglass.core.notif.NotificationFilter

/** Debug: a notification on the channel the filter lets through, with a plain action and a reply action. */
object TestNotification {
    private var counter = 1_000   // ids clear of the service notification (1, 2)

    fun post(context: Context) {
        val nm = context.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(NotificationFilter.TEST_CHANNEL, "Glass test", NotificationManager.IMPORTANCE_HIGH))
        val id = ++counter
        val ping = PendingIntent.getBroadcast(context, id, Intent(context, TestActionReceiver::class.java).putExtra("id", id),
            PendingIntent.FLAG_IMMUTABLE)
        val reply = PendingIntent.getBroadcast(context, 100_000 + id, Intent(context, TestActionReceiver::class.java).putExtra("id", id),
            PendingIntent.FLAG_MUTABLE)
        val n = Notification.Builder(context, NotificationFilter.TEST_CHANNEL)
            .setSmallIcon(android.R.drawable.ic_dialog_email)
            .setContentTitle("Test $id")
            .setContentText("Hello from OpenMyGlass, message $id")
            .setCategory(Notification.CATEGORY_MESSAGE)
            .addAction(Notification.Action.Builder(null, "Ping", ping).build())
            .addAction(Notification.Action.Builder(null, "Reply", reply)
                .addRemoteInput(RemoteInput.Builder("reply").setLabel("Reply").build()).build())
            .build()
        nm.notify(id, n)
    }
}

class TestActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val id = intent.getIntExtra("id", -1)
        val reply = RemoteInput.getResultsFromIntent(intent)?.getCharSequence("reply")
        if (reply == null) {
            Log.i("TestNotification", "test action fired for $id")
            Toast.makeText(context, "Glass action: Ping $id", Toast.LENGTH_SHORT).show()
            return
        }
        // Like a messaging app: show the reply in the notification (debug channel only; nothing is logged)
        val nm = context.getSystemService(NotificationManager::class.java)
        nm.notify(id, Notification.Builder(context, NotificationFilter.TEST_CHANNEL)
            .setSmallIcon(android.R.drawable.ic_dialog_email)
            .setContentTitle("Test $id")
            .setContentText("Replied: $reply")
            .setOnlyAlertOnce(true)
            .build())
    }
}
