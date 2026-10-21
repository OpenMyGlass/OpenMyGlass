package org.openmyglass.core.location

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.openmyglass.protocol.v1.Bridge
import org.openmyglass.protocol.v1.Bridge.Frame
import java.util.Collections
import kotlin.time.Duration.Companion.milliseconds
import kotlin.time.Duration.Companion.seconds

class LocationRelayTest {
    /** Fake phone location: records requests, lets the test push fixes, can refuse to start. */
    private class FakeSource : LocationSource {
        val requests = Collections.synchronizedList(mutableListOf<LocationRequest>())
        @Volatile var callback: LocationCallback? = null
        @Volatile var running = 0
        @Volatile var refuse: String? = null
        @Volatile var initialFix: LocationFix? = null
        override fun start(request: LocationRequest, callback: LocationCallback): AutoCloseable {
            refuse?.let { throw LocationUnavailableException(it) }
            requests += request
            this.callback = callback
            running++
            initialFix?.let(callback::onFix)
            return AutoCloseable { running-- }
        }
    }

    private val source = FakeSource()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val relay = LocationRelay(source, scope, noFixTimeout = 300.milliseconds, retryEvery = 200.milliseconds)
    private val sent = Collections.synchronizedList(mutableListOf<Frame>())
    private val send: suspend (Frame) -> Unit = { sent += it }

    @AfterEach
    fun tearDown() = scope.cancel()

    private fun subscribe(intervalMs: Int, high: Boolean = false) = relay.onSubscribe(
        Bridge.LocationSubscribe.newBuilder().setIntervalMs(intervalMs).setHighAccuracy(high).build(), send)

    private val fix = LocationFix(45.4642, 9.19, accuracyM = 8f, altitudeM = 120.5, bearingDeg = 90f, speedMps = null,
        fixTimeMs = 1_790_000_000_000, provider = "gps")

    private fun await(what: String, condition: () -> Boolean) {
        val until = System.nanoTime() + 3.seconds.inWholeNanoseconds
        while (!condition()) {
            check(System.nanoTime() < until) { "timed out waiting for $what; sent: $sent" }
            Thread.sleep(10)
        }
    }

    @Test
    fun `nothing runs until glass subscribes`() {
        Thread.sleep(100)
        assertEquals(0, source.running)
        assertTrue(sent.isEmpty())
    }

    @Test
    fun `a subscription starts the source and fixes reach glass`() {
        subscribe(5_000, high = true)
        assertEquals(listOf(LocationRequest(5_000, highAccuracy = true)), source.requests)
        source.callback!!.onFix(fix)
        await("update") { sent.any { it.hasLocationUpdate() } }
        val u = sent.single { it.hasLocationUpdate() }.locationUpdate
        assertEquals(45.4642, u.latitudeDeg)
        assertEquals(9.19, u.longitudeDeg)
        assertEquals(8f, u.accuracyM)
        assertTrue(u.hasAltitudeM()); assertEquals(120.5, u.altitudeM)
        assertTrue(u.hasBearingDeg()); assertEquals(90f, u.bearingDeg)
        assertFalse(u.hasSpeedMps())
        assertEquals(1_790_000_000_000, u.fixTimeMs)
        assertEquals("gps", u.provider)
    }

    @Test
    fun `fixes keep their order`() {
        subscribe(1_000)
        repeat(10) { i -> source.callback!!.onFix(fix.copy(fixTimeMs = i.toLong())) }
        await("ten updates") { sent.size == 10 }
        assertEquals((0L until 10L).toList(), sent.map { it.locationUpdate.fixTimeMs })
    }

    @Test
    fun `interval 0 stops the source and later fixes are dropped`() {
        subscribe(2_000)
        val old = source.callback!!
        subscribe(0)
        assertEquals(0, source.running)
        old.onFix(fix)
        Thread.sleep(100)
        assertTrue(sent.none { it.hasLocationUpdate() })
    }

    @Test
    fun `a new subscription replaces the old one`() {
        subscribe(10_000)
        val old = source.callback!!
        subscribe(2_000, high = true)
        assertEquals(1, source.running)
        assertEquals(LocationRequest(2_000, true), source.requests.last())
        old.onFix(fix.copy(provider = "network"))
        source.callback!!.onFix(fix)
        await("update") { sent.any { it.hasLocationUpdate() } }
        Thread.sleep(50)
        assertEquals(listOf("gps"), sent.filter { it.hasLocationUpdate() }.map { it.locationUpdate.provider })
    }

    @Test
    fun `the session ending stops the source`() {
        subscribe(2_000)
        relay.sessionEnded()
        assertEquals(0, source.running)
    }

    @Test
    fun `too short an interval is raised to the minimum`() {
        subscribe(10)
        assertEquals(LocationRelay.MIN_INTERVAL_MS, source.requests.single().intervalMs)
    }

    @Test
    fun `a fix delivered during start is sent`() {
        source.initialFix = fix
        subscribe(5_000)
        await("update") { sent.any { it.hasLocationUpdate() } }
        Thread.sleep(400)   // past the no-fix timeout: a fix arrived, so no "no-fix"
        assertTrue(sent.none { it.hasLocationUnavailable() })
    }

    @Test
    fun `without permission glass is told once and the phone retries while subscribed`() {
        source.refuse = LocationUnavailable.PERMISSION
        subscribe(5_000)
        await("unavailable") { sent.any { it.hasLocationUnavailable() } }
        assertEquals("permission", sent.single().locationUnavailable.reason)
        Thread.sleep(500)   // two retries, still refused: not repeated
        assertEquals(1, sent.size)
        source.refuse = null
        await("retry starts the source") { source.running == 1 }
        source.callback!!.onFix(fix)
        await("update") { sent.any { it.hasLocationUpdate() } }
    }

    @Test
    fun `no retry after the subscription ended`() {
        source.refuse = LocationUnavailable.DISABLED
        subscribe(5_000)
        relay.sessionEnded()
        source.refuse = null
        Thread.sleep(400)
        assertEquals(0, source.running)
    }

    @Test
    fun `no fix in time sends no-fix, and a fix after it still goes out`() {
        subscribe(5_000)
        await("no-fix") { sent.any { it.hasLocationUnavailable() } }
        assertEquals("no-fix", sent.single().locationUnavailable.reason)
        source.callback!!.onFix(fix)
        await("update") { sent.any { it.hasLocationUpdate() } }
    }

    @Test
    fun `the source reporting location turned off reaches glass`() {
        subscribe(5_000)
        source.callback!!.onFix(fix)
        source.callback!!.onUnavailable(LocationUnavailable.DISABLED)
        await("unavailable") { sent.any { it.hasLocationUnavailable() } }
        assertEquals("disabled", sent.last().locationUnavailable.reason)
    }

    @Test
    fun `a failing send does not stop the feed`() {
        var failures = 1
        relay.onSubscribe(Bridge.LocationSubscribe.newBuilder().setIntervalMs(1_000).build()) {
            if (failures-- > 0) throw java.io.IOException("link down")
            sent += it
        }
        source.callback!!.onFix(fix.copy(fixTimeMs = 1))
        source.callback!!.onFix(fix.copy(fixTimeMs = 2))
        await("second update") { sent.size == 1 }
        assertEquals(2L, sent.single().locationUpdate.fixTimeMs)
    }
}
