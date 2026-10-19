package org.openmyglass.core.companion

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.openmyglass.core.framing.DelimitedFraming
import org.openmyglass.core.transport.Channel
import org.openmyglass.core.transport.readFrameWithTimeout
import org.openmyglass.protocol.companion.Companion.AndroidDeviceInfo
import org.openmyglass.protocol.companion.Companion.CompanionDeviceInfo
import org.openmyglass.protocol.companion.Companion.CompanionFeatureInfo
import org.openmyglass.protocol.companion.Companion.Envelope
import org.openmyglass.protocol.companion.Companion.GlassInfoRequest
import org.openmyglass.protocol.companion.Companion.GlassInfoResponse
import java.io.IOException
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds

/**
 * One session of the original companion protocol over a [Channel] (see docs/protocol/handshake.md).
 * Glass speaks first: [handshake] reads Glass's envelope, then answers with ours.
 */
class CompanionLink(
    private val channel: Channel,
    private val phone: () -> PhoneState,
    private val clock: () -> Long = System::currentTimeMillis,
    private val uptime: () -> Long = { System.nanoTime() / 1_000_000 },
) {
    private val writeLock = Mutex()

    suspend fun handshake(timeout: Duration = 10.seconds): GlassHello {
        val glassEnvelope = try {
            readWithTimeout(timeout) ?: throw IOException("Glass closed the link during the handshake")
        } catch (e: IOException) {
            close()
            throw e
        }
        val state = phone()
        send(
            newEnvelope()
                .setAndroidDeviceInfo(AndroidDeviceInfo.newBuilder().setSdkVersion(state.sdkVersion))
                .setCompanionDeviceInfoC2G(
                    CompanionDeviceInfo.newBuilder()
                        .setWifiConnected(state.wifiConnected)
                        .setIsInteractiveState(state.interactive)
                        .setGpsEnabled(state.gpsEnabled)
                )
                .setCompanionFeatureInfoC2G(
                    CompanionFeatureInfo.newBuilder()
                        .setIsPhotoSyncEnabled(state.photoSyncEnabled)
                        .setIsKeyboardTextEntrySupported(state.keyboardTextEntrySupported)
                )
                .setTimezoneC2G(state.timeZoneId)
        )
        return GlassHello(
            remoteVersion = glassEnvelope.version,
            glassSdkVersion = if (glassEnvelope.hasAndroidDeviceInfo()) glassEnvelope.androidDeviceInfo.sdkVersion else null,
            wallpaperFeatureEnabled = glassEnvelope.glassFeatureInfoG2C.isWallpaperFeatureEnabled,
        )
    }

    suspend fun requestGlassInfo() = send(
        newEnvelope().setGlassInfoRequestC2G(
            GlassInfoRequest.newBuilder()
                .setRequestBatteryLevel(true)
                .setRequestStorageInfo(true)
                .setRequestDeviceName(true)
                .setRequestSoftwareVersion(true)
                .setRequestNeedSetup(true)
                .setRequestScreenDimensions(true)
        )
    )

    suspend fun sendTimeZone(id: String) = send(newEnvelope().setTimezoneC2G(id))

    /** Envelopes from Glass until a clean end of stream (completes) or an I/O error (throws). */
    fun events(): Flow<CompanionEvent> = flow {
        while (true) {
            val envelope = withContext(Dispatchers.IO) { readEnvelope() } ?: break
            emit(toEvent(envelope))
        }
    }

    fun close() {
        runCatching { channel.close() }
    }

    private suspend fun send(builder: Envelope.Builder) {
        val bytes = builder.build().toByteArray()
        writeLock.withLock {
            withContext(Dispatchers.IO) { DelimitedFraming.writeFrame(channel.output, bytes) }
        }
    }

    private fun newEnvelope(): Envelope.Builder = Envelope.newBuilder()
        .setVersion(CompanionConstants.VERSION)
        .setTimeMillis(clock())
        .setUptimeMillis(uptime())

    private fun readEnvelope(): Envelope? = DelimitedFraming.readFrame(channel.input)?.let(Envelope::parseFrom)

    private suspend fun readWithTimeout(timeout: Duration): Envelope? =
        channel.readFrameWithTimeout(timeout, "handshake")?.let(Envelope::parseFrom)

    private fun toEvent(envelope: Envelope): CompanionEvent =
        if (envelope.hasGlassInfoResponseG2C()) CompanionEvent.GlassInfoReceived(envelope.glassInfoResponseG2C.toGlassInfo())
        else CompanionEvent.Other(envelope)

    private fun GlassInfoResponse.toGlassInfo() = GlassInfo(
        batteryLevel = if (hasBatteryLevel()) batteryLevel else null,
        storageTotalBytes = if (hasExternalStorageTotalBytes()) externalStorageTotalBytes else null,
        storageAvailableBytes = if (hasExternalStorageAvailableBytes()) externalStorageAvailableBytes else null,
        deviceName = if (hasDeviceName()) deviceName else null,
        softwareVersion = if (hasSoftwareVersion()) softwareVersion else null,
        needSetup = if (hasNeedSetup()) needSetup else null,
        screenWidthPx = if (hasGlassScreenWidthPixels()) glassScreenWidthPixels else null,
        screenHeightPx = if (hasGlassScreenHeightPixels()) glassScreenHeightPixels else null,
    )
}
