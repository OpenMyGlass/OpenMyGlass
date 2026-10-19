package org.openmyglass.android.service

import android.content.Context
import android.location.LocationManager
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Build
import android.os.PowerManager
import org.openmyglass.core.companion.PhoneState
import java.util.TimeZone

/** What the phone tells the XE24 firmware in the companion handshake. */
object PhoneStates {
    fun current(context: Context): PhoneState {
        val cm = context.getSystemService(ConnectivityManager::class.java)
        val wifi = cm?.getNetworkCapabilities(cm.activeNetwork)?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true
        val interactive = context.getSystemService(PowerManager::class.java)?.isInteractive == true
        val gps = runCatching {
            context.getSystemService(LocationManager::class.java)?.isProviderEnabled(LocationManager.GPS_PROVIDER) == true
        }.getOrDefault(false)
        return PhoneState(Build.VERSION.SDK_INT, TimeZone.getDefault().id, wifi, interactive, gps)
    }
}
