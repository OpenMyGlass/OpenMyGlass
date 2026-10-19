package org.openmyglass.core.companion

import kotlinx.coroutines.async
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.assertThrows
import org.openmyglass.protocol.companion.Companion.Envelope
import org.openmyglass.protocol.companion.Companion.GlassInfoResponse
import java.io.IOException
import kotlin.time.Duration.Companion.milliseconds
import kotlin.time.Duration.Companion.seconds

class CompanionLinkTest {

    private val glass = FakeGlass.open()
    private val phone = PhoneState(
        sdkVersion = 34, timeZoneId = "Europe/Rome", wifiConnected = true, interactive = true, gpsEnabled = true,
    )
    private val link = CompanionLink(glass.phoneChannel, phone = { phone }, clock = { 1_000L }, uptime = { 42L })

    @AfterEach
    fun tearDown() = glass.close()

    @Test
    fun `handshake waits for glass then answers`() = runBlocking {
        glass.send(FakeGlass.glassHello(sdk = 19))
        val hello = link.handshake()
        assertEquals(GlassHello(655366, 19, false), hello)

        val ours = glass.receive()
        assertEquals(655366, ours.version)
        assertEquals(1_000L, ours.timeMillis)
        assertEquals(42L, ours.uptimeMillis)
        assertEquals(34, ours.androidDeviceInfo.sdkVersion)
        assertEquals("Europe/Rome", ours.timezoneC2G)
        assertTrue(ours.companionDeviceInfoC2G.gpsEnabled)
        assertTrue(ours.companionDeviceInfoC2G.wifiConnected)
    }

    @Test
    fun `handshake does not send before glass speaks`() = runBlocking {
        val pending = async { link.handshake() }
        assertTrue(glass.silentFor(200), "phone must wait for Glass's handshake")
        glass.send(FakeGlass.glassHello())
        pending.await()
        glass.receive()
        Unit
    }

    @Test
    fun `handshake times out on a silent glass`() = runBlocking {
        assertThrows<IOException> { runBlocking { link.handshake(timeout = 500.milliseconds) } }
        assertThrows<IOException> { glass.phoneChannel.output.write(1) }
        Unit
    }

    @Test
    fun `glass info request and response`() = runBlocking {
        glass.send(FakeGlass.glassHello()); link.handshake(); glass.receive()

        link.requestGlassInfo()
        val req = glass.receive().glassInfoRequestC2G
        assertTrue(req.requestBatteryLevel && req.requestStorageInfo && req.requestDeviceName &&
            req.requestSoftwareVersion && req.requestNeedSetup && req.requestScreenDimensions)

        glass.send(Envelope.newBuilder().setVersion(655366).setGlassInfoResponseG2C(
            GlassInfoResponse.newBuilder().setBatteryLevel(80).setDeviceName("Glass")
                .setGlassScreenWidthPixels(640).setGlassScreenHeightPixels(360)
        ).build())
        val event = withTimeout(2.seconds) { link.events().first() }
        assertEquals(
            CompanionEvent.GlassInfoReceived(GlassInfo(80, null, null, "Glass", null, null, 640, 360)),
            event,
        )
    }

    @Test
    fun `time zone is sent`() = runBlocking {
        glass.send(FakeGlass.glassHello()); link.handshake(); glass.receive()
        link.sendTimeZone("America/New_York")
        assertEquals("America/New_York", glass.receive().timezoneC2G)
    }

    @Test
    fun `events complete on clean disconnect`() = runBlocking {
        glass.send(FakeGlass.glassHello()); link.handshake(); glass.receive()
        glass.closeOutput()
        assertEquals(emptyList<CompanionEvent>(), withTimeout(2.seconds) { link.events().toList() })
    }
}
