package org.openmyglass.core.location

/** One position fix from the phone (docs/location.md). Optional values are null when the provider gave none. */
data class LocationFix(
    val latitudeDeg: Double,
    val longitudeDeg: Double,
    /** Horizontal accuracy (68 % radius); 0 = unknown. */
    val accuracyM: Float,
    val altitudeM: Double? = null,
    val bearingDeg: Float? = null,
    val speedMps: Float? = null,
    /** UTC milliseconds of the fix. */
    val fixTimeMs: Long,
    /** "gps", "network" or "fused". */
    val provider: String,
)

data class LocationRequest(val intervalMs: Long, val highAccuracy: Boolean)

interface LocationCallback {
    fun onFix(fix: LocationFix)
    /** The running request cannot deliver fixes for now (e.g. location turned off); one of [LocationUnavailable]. */
    fun onUnavailable(reason: String)
}

/** Reasons sent in the bridge's LocationUnavailable. */
object LocationUnavailable {
    const val PERMISSION = "permission"
    const val DISABLED = "disabled"
    const val NO_FIX = "no-fix"
}

/** The request cannot start at all; [reason] is one of [LocationUnavailable]. */
class LocationUnavailableException(val reason: String) : Exception(reason)

/** The phone's location (Android LocationManager on the phone, a fake in tests). Callbacks may come from any thread. */
fun interface LocationSource {
    /** Starts updates; closing the result stops them. Throws [LocationUnavailableException] when it cannot start. */
    fun start(request: LocationRequest, callback: LocationCallback): AutoCloseable
}
