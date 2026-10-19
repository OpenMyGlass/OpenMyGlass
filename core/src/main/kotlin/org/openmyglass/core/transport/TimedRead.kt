package org.openmyglass.core.transport

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.openmyglass.core.framing.DelimitedFraming
import java.io.IOException
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.time.Duration

/**
 * Reads one frame, or fails after [timeout]. Blocking socket reads ignore coroutine cancellation,
 * so the timeout closes the channel to unblock the read.
 */
suspend fun Channel.readFrameWithTimeout(timeout: Duration, what: String = "read"): ByteArray? = coroutineScope {
    val timedOut = AtomicBoolean(false)
    val watchdog = launch { delay(timeout); timedOut.set(true); runCatching { close() } }
    try {
        withContext(Dispatchers.IO) { DelimitedFraming.readFrame(input) }
    } catch (e: IOException) {
        throw if (timedOut.get()) IOException("$what timed out after $timeout") else e
    } finally {
        watchdog.cancel()
    }
}
