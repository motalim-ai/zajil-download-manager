package io.github.motalim_ai.zajil.downloads

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Each running download's speed, in bytes per second, from its progress
 * reports: measured over at least half a second and smoothed, so that the
 * screen's number does not jump with every report. Only in memory.
 */
class Speedometer(private val now: () -> Long = { System.nanoTime() / 1_000_000 }) {
    private class Mark(val at: Long, val done: Long, val rate: Double)

    private val marks = HashMap<Long, Mark>()
    private val _rates = MutableStateFlow<Map<Long, Long>>(emptyMap())
    val rates: StateFlow<Map<Long, Long>> = _rates.asStateFlow()

    /** Download [id] has [done] bytes now. */
    @Synchronized
    fun report(id: Long, done: Long) {
        val at = now()
        val last = marks[id]
        if (last == null || done < last.done) {
            // The first report, or it started over: no speed yet.
            marks[id] = Mark(at, done, 0.0)
            return
        }
        val elapsed = at - last.at
        if (elapsed < 500) return
        val sample = (done - last.done) * 1000.0 / elapsed
        val rate = if (last.rate == 0.0) sample else last.rate * 0.6 + sample * 0.4
        marks[id] = Mark(at, done, rate)
        publish()
    }

    /** Forgets every download but [running]. */
    @Synchronized
    fun keepOnly(running: Set<Long>) {
        if (marks.keys.all { it in running }) return
        marks.keys.retainAll(running)
        publish()
    }

    private fun publish() {
        _rates.value = marks.mapValues { it.value.rate.toLong() }.filterValues { it > 0 }
    }
}
