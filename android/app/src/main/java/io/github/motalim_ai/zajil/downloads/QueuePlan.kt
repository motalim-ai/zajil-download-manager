package io.github.motalim_ai.zajil.downloads

import java.time.DayOfWeek
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.ZoneId
import java.time.ZonedDateTime

/**
 * A queue (A2; several from 0.3.0, as the approved design's «Night queue»):
 * its own [rules] and [name]. [MAIN] is the main queue, which the app
 * names and which is never deleted.
 */
data class Queue(val id: Long, val name: String, val rules: QueueRules) {
    val main get() = id == MAIN

    companion object {
        const val MAIN = 0L
    }
}

/** A queue's settings (A2), kept in the app's preferences ([QueueSettings]). */
data class QueueRules(
    /** How many downloads run at once, at least 1. */
    val maxRunning: Int = 3,
    /** Only on an unmetered network (Wi-Fi). */
    val wifiOnly: Boolean = false,
    /** The queue starts at [startAt] on each of [days]. */
    val scheduled: Boolean = false,
    val startAt: LocalTime = LocalTime.of(2, 0),
    /** The days it starts on (D5's night queue): every day unless the user leaves some out; never none. */
    val days: Set<DayOfWeek> = DayOfWeek.entries.toSet(),
    /**
     * With [scheduled]: the queue has reached its time and runs until it is
     * empty; then it waits for the next [startAt].
     */
    val open: Boolean = false,
)

/** The network as the queue needs it. */
data class NetworkState(val connected: Boolean, val unmetered: Boolean) {
    fun allows(rules: QueueRules) = connected && (unmetered || !rules.wifiOnly)
}

/** Why a queued download has not started yet, for the screen. */
enum class Wait { Turn, Schedule, Network, Wifi }

object QueuePlan {
    /**
     * Which queued downloads start now, in queue order: as many as there is
     * room for under [QueueRules.maxRunning], when the network and the
     * schedule allow.
     */
    fun toStart(list: List<Download>, rules: QueueRules, network: NetworkState): List<Long> {
        if (!network.allows(rules)) return emptyList()
        if (rules.scheduled && !rules.open) return emptyList()
        val room = rules.maxRunning.coerceAtLeast(1) - list.count { it.status == Status.Running }
        if (room <= 0) return emptyList()
        return queued(list).take(room).map { it.id }
    }

    /**
     * Which queued downloads start now, each queue by its own rules: a
     * download of a queue that is gone counts as the main queue's.
     */
    fun toStart(list: List<Download>, queues: List<Queue>, network: NetworkState): List<Long> =
        queues.flatMap { queue -> toStart(list.filter { queueOf(it, queues) == queue.id }, queue.rules, network) }

    /** [download]'s queue among [queues]: its own, or the main one if its own is gone. */
    fun queueOf(download: Download, queues: List<Queue>): Long =
        download.queue.takeIf { id -> queues.any { it.id == id } } ?: Queue.MAIN

    /** The queued downloads in the order they start. */
    fun queued(list: List<Download>): List<Download> =
        list.filter { it.status == Status.Queued }.sortedWith(compareBy(Download::order, Download::id))

    fun waitOf(rules: QueueRules, network: NetworkState): Wait = when {
        rules.scheduled && !rules.open -> Wait.Schedule
        !network.connected -> Wait.Network
        rules.wifiOnly && !network.unmetered -> Wait.Wifi
        else -> Wait.Turn
    }

    /**
     * The next time the clock shows [at] in [zone] on one of [days],
     * strictly after [now] (a time that a clock change skips is moved
     * forward, as java.time does). No day at all counts as every day.
     */
    fun nextStart(now: ZonedDateTime, at: LocalTime, zone: ZoneId = now.zone, days: Set<DayOfWeek> = DayOfWeek.entries.toSet()): ZonedDateTime {
        val allowed = days.ifEmpty { DayOfWeek.entries.toSet() }
        val today = ZonedDateTime.of(LocalDateTime.of(now.toLocalDate(), at), zone)
        var next = if (today.isAfter(now)) today else ZonedDateTime.of(LocalDateTime.of(now.toLocalDate().plusDays(1), at), zone)
        while (next.dayOfWeek !in allowed) next = ZonedDateTime.of(LocalDateTime.of(next.toLocalDate().plusDays(1), at), zone)
        return next
    }

    /**
     * The queue order after moving [id] one place earlier among the queued
     * downloads: it swaps places with the one before it.
     */
    fun movedUp(list: List<Download>, id: Long): List<Download> {
        val queue = queued(list)
        val at = queue.indexOfFirst { it.id == id }
        if (at <= 0) return list
        val (before, mine) = queue[at - 1] to queue[at]
        return list.map {
            when (it.id) {
                mine.id -> it.copy(order = before.order)
                before.id -> it.copy(order = mine.order)
                else -> it
            }
        }
    }
}
