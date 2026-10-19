package org.openmyglass.core.connection

import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds

/**
 * Waits on the full transition history recorded by an `onStateChange` callback, in order: each [await] returns the
 * next recorded state matching its predicate. Unlike a conflated StateFlow, short-lived states are never missed.
 */
class HistoryCursor<T>(private val history: MutableList<T>) {
    private var next = 0

    fun await(timeout: Duration = 3.seconds, predicate: (T) -> Boolean): T {
        val deadline = System.nanoTime() + timeout.inWholeNanoseconds
        while (true) {
            synchronized(history) {
                for (i in next until history.size) {
                    if (predicate(history[i])) { next = i + 1; return history[i] }
                }
            }
            if (System.nanoTime() > deadline) throw AssertionError("no matching state in $timeout; history: $history")
            Thread.sleep(5)
        }
    }
}
