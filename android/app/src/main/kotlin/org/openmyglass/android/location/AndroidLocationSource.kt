package org.openmyglass.android.location

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Build
import android.os.Bundle
import android.os.Looper
import org.openmyglass.core.location.LocationCallback
import org.openmyglass.core.location.LocationFix
import org.openmyglass.core.location.LocationRequest
import org.openmyglass.core.location.LocationSource
import org.openmyglass.core.location.LocationUnavailable
import org.openmyglass.core.location.LocationUnavailableException

/**
 * The phone's location from the platform [LocationManager] (no Play services): GPS for a high-accuracy request,
 * otherwise the fused provider (Android 12+) or the network provider. Runs only between [start] and close, i.e. only
 * while Glass is subscribed (docs/location.md).
 */
class AndroidLocationSource(context: Context) : LocationSource {
    private val app = context.applicationContext

    @SuppressLint("MissingPermission")   // checked first; a late revoke throws SecurityException, handled by the relay
    override fun start(request: LocationRequest, callback: LocationCallback): AutoCloseable {
        val fine = granted(Manifest.permission.ACCESS_FINE_LOCATION)
        if (!fine && !granted(Manifest.permission.ACCESS_COARSE_LOCATION)) throw LocationUnavailableException(LocationUnavailable.PERMISSION)
        val manager = app.getSystemService(LocationManager::class.java)
            ?: throw LocationUnavailableException(LocationUnavailable.DISABLED)
        if (!manager.isLocationEnabled) throw LocationUnavailableException(LocationUnavailable.DISABLED)
        val provider = chooseProvider(manager, request.highAccuracy, fine)
            ?: throw LocationUnavailableException(LocationUnavailable.DISABLED)
        val listener = object : LocationListener {
            override fun onLocationChanged(location: Location) = callback.onFix(location.toFix(provider))
            override fun onProviderDisabled(p: String) = callback.onUnavailable(LocationUnavailable.DISABLED)
            override fun onProviderEnabled(p: String) {}
            // Abstract on API 29 (default methods arrived in API 30): implemented so the call cannot fail there
            @Deprecated("Deprecated in Java")
            override fun onStatusChanged(p: String?, status: Int, extras: Bundle?) {}
        }
        if (Build.VERSION.SDK_INT >= 31) {
            val quality = if (request.highAccuracy) android.location.LocationRequest.QUALITY_HIGH_ACCURACY
                else android.location.LocationRequest.QUALITY_BALANCED_POWER_ACCURACY
            manager.requestLocationUpdates(provider,
                android.location.LocationRequest.Builder(request.intervalMs).setQuality(quality).build(),
                app.mainExecutor, listener)
        } else {
            manager.requestLocationUpdates(provider, request.intervalMs, 0f, listener, Looper.getMainLooper())
        }
        // A recent last fix gives Glass a position right away instead of after the first new fix
        manager.getLastKnownLocation(provider)
            ?.takeIf { System.currentTimeMillis() - it.time in 0..LAST_FIX_MAX_AGE_MS }
            ?.let { callback.onFix(it.toFix(provider)) }
        return AutoCloseable { manager.removeUpdates(listener) }
    }

    private fun granted(permission: String) = app.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    private fun chooseProvider(manager: LocationManager, highAccuracy: Boolean, fine: Boolean): String? {
        val enabled = manager.getProviders(true).toSet()
        val fused = if (Build.VERSION.SDK_INT >= 31) LocationManager.FUSED_PROVIDER else null
        val order = if (highAccuracy && fine) listOf(LocationManager.GPS_PROVIDER, fused, LocationManager.NETWORK_PROVIDER)
            else listOf(fused, LocationManager.NETWORK_PROVIDER, LocationManager.GPS_PROVIDER.takeIf { fine })
        return order.filterNotNull().firstOrNull { it in enabled }
    }

    private fun Location.toFix(requested: String) = LocationFix(
        latitudeDeg = latitude,
        longitudeDeg = longitude,
        accuracyM = if (hasAccuracy()) accuracy else 0f,
        altitudeM = if (hasAltitude()) altitude else null,
        bearingDeg = if (hasBearing()) bearing else null,
        speedMps = if (hasSpeed()) speed else null,
        fixTimeMs = time,
        provider = provider ?: requested,
    )

    companion object {
        private const val LAST_FIX_MAX_AGE_MS = 2 * 60 * 1000L

        fun hasPermission(context: Context) =
            context.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED ||
                context.checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION) == PackageManager.PERMISSION_GRANTED
    }
}
