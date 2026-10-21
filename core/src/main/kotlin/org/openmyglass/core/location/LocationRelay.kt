package org.openmyglass.core.location

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.openmyglass.protocol.v1.Bridge
import org.openmyglass.protocol.v1.Bridge.Frame
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds

/**
 * Phone side of the bridge's location feed (docs/location.md): the phone's [source] runs only while Glass holds a
 * subscription. A new LocationSubscribe replaces the previous one, interval 0 stops it, and [sessionEnded] ends it.
 * If the source cannot start (no permission, location off) Glass gets LocationUnavailable and the phone retries every
 * [retryEvery] while the subscription lasts; with no fix within [noFixTimeout] of starting, Glass gets "no-fix".
 * Fixes are passed through, never stored or logged.
 */
class LocationRelay(
    private val source: LocationSource,
    private val scope: CoroutineScope,
    private val noFixTimeout: Duration = 60.seconds,
    private val retryEvery: Duration = 60.seconds,
    private val minIntervalMs: Long = MIN_INTERVAL_MS,
) {
    private inner class Subscription(val session: Any, val request: LocationRequest, val send: suspend (Frame) -> Unit) : LocationCallback {
        // Frames go out in order from one coroutine; a stalled link drops the oldest rather than piling up
        val outbox = Channel<Frame>(16, BufferOverflow.DROP_OLDEST)
        val pump: Job = scope.launch {
            for (frame in outbox) {
                try {
                    send(frame)
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Throwable) {
                    // The link failed: BridgeManager ends the session, which ends this subscription
                }
            }
        }
        var handle: AutoCloseable? = null
        var timer: Job? = null
        var gotFix = false
        var lastReason: String? = null

        override fun onFix(fix: LocationFix) {
            synchronized(lock) {
                if (current !== this) return
                gotFix = true
                lastReason = null
                timer?.cancel()
                outbox.trySend(update(fix))
            }
        }

        override fun onUnavailable(reason: String) {
            synchronized(lock) { if (current === this) unavailable(reason) }
        }

        /** Caller holds the lock. Sends a reason once, not again until a fix or another reason came in between. */
        fun unavailable(reason: String) {
            if (reason == lastReason) return
            lastReason = reason
            outbox.trySend(Frame.newBuilder().setLocationUnavailable(Bridge.LocationUnavailable.newBuilder().setReason(reason)).build())
        }
    }

    private val lock = Any()
    private var current: Subscription? = null

    /** Glass sent a LocationSubscribe on [session] (any identity token, e.g. its link), which [send] writes to. */
    fun onSubscribe(s: Bridge.LocationSubscribe, session: Any, send: suspend (Frame) -> Unit) {
        synchronized(lock) {
            stop()
            if (s.intervalMs == 0) return
            val interval = s.intervalMs.toUInt().toLong().coerceAtLeast(minIntervalMs)
            tryStart(Subscription(session, LocationRequest(interval, s.highAccuracy), send).also { current = it })
        }
    }

    /**
     * [session] ended: its subscription ends with it. A newer session's subscription is kept (it may have subscribed
     * before the old session finished cleaning up).
     */
    fun sessionEnded(session: Any) {
        synchronized(lock) { if (current?.session === session) stop() }
    }

    /** Caller holds the lock. */
    private fun tryStart(sub: Subscription) {
        try {
            // The source may call back synchronously (on this thread, so the lock is reentrant): current is already set
            sub.handle = source.start(sub.request, sub)
        } catch (e: Exception) {
            // SecurityException: the permission was revoked between the source's check and its request
            sub.unavailable((e as? LocationUnavailableException)?.reason ?: LocationUnavailable.PERMISSION.takeIf { e is SecurityException }
                ?: LocationUnavailable.DISABLED)
            sub.timer = scope.launch {
                delay(retryEvery)
                synchronized(lock) { if (current === sub) tryStart(sub) }
            }
            return
        }
        if (!sub.gotFix) sub.timer = scope.launch {
            delay(noFixTimeout)
            // Not while a cause is already reported (location off, no permission): no-fix would hide it
            synchronized(lock) {
                if (current === sub && !sub.gotFix && sub.lastReason == null) sub.unavailable(LocationUnavailable.NO_FIX)
            }
        }
    }

    /** Caller holds the lock. */
    private fun stop() {
        current?.let { sub ->
            sub.timer?.cancel()
            runCatching { sub.handle?.close() }
            sub.outbox.close()
            sub.pump.cancel()
        }
        current = null
    }

    private fun update(fix: LocationFix): Frame = Frame.newBuilder().setLocationUpdate(
        Bridge.LocationUpdate.newBuilder()
            .setLatitudeDeg(fix.latitudeDeg)
            .setLongitudeDeg(fix.longitudeDeg)
            .setAccuracyM(fix.accuracyM)
            .setFixTimeMs(fix.fixTimeMs)
            .setProvider(fix.provider)
            .also { b -> fix.altitudeM?.let(b::setAltitudeM) }
            .also { b -> fix.bearingDeg?.let(b::setBearingDeg) }
            .also { b -> fix.speedMps?.let(b::setSpeedMps) }
    ).build()

    companion object {
        /** Faster than once a second is not useful on Glass and costs the phone's battery. */
        const val MIN_INTERVAL_MS = 1000L
    }
}
