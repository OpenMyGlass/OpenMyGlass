package org.openmyglass.core.connection

import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds

/** Exponential reconnection delay: initial, initial × factor, … capped at [max]. */
class Backoff(
    val initial: Duration = 1.seconds,
    val max: Duration = 60.seconds,
    val factor: Double = 2.0,
) {
    private var current: Duration? = null

    fun next(): Duration {
        val next = current?.let { minOf(it * factor, max) } ?: initial
        current = next
        return next
    }

    fun reset() {
        current = null
    }
}
