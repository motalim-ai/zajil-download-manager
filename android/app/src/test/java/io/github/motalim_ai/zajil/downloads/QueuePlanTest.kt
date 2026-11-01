package io.github.motalim_ai.zajil.downloads

import java.time.LocalTime
import java.time.ZoneId
import java.time.ZonedDateTime
import org.junit.Assert.assertEquals
import org.junit.Test

class QueuePlanTest {
    private val online = NetworkState(connected = true, unmetered = false)
    private val wifi = NetworkState(connected = true, unmetered = true)
    private val offline = NetworkState(connected = false, unmetered = false)

    private fun queued(id: Long, order: Long = id) = Download(id, "https://example.com/$id", Status.Queued, order = order)

    private fun running(id: Long) = Download(id, "https://example.com/$id", Status.Running)

    @Test
    fun at_most_the_limit_in_queue_order() {
        val list = (1L..5L).map { queued(it) }
        assertEquals(listOf(1L, 2L), QueuePlan.toStart(list, QueueRules(maxRunning = 2), online))
    }

    @Test
    fun running_downloads_take_their_room() {
        val list = listOf(running(1), queued(2), queued(3), queued(4))
        assertEquals(listOf(2L), QueuePlan.toStart(list, QueueRules(maxRunning = 2), online))
        assertEquals(emptyList<Long>(), QueuePlan.toStart(list + running(5), QueueRules(maxRunning = 2), online))
    }

    @Test
    fun the_order_not_the_id_decides() {
        val list = listOf(queued(1, order = 9), queued(2, order = 3), queued(3, order = 5))
        assertEquals(listOf(2L, 3L), QueuePlan.toStart(list, QueueRules(maxRunning = 2), online))
    }

    @Test
    fun wifi_only_waits_on_mobile_data_and_starts_on_wifi() {
        val list = listOf(queued(1))
        val rules = QueueRules(wifiOnly = true)
        assertEquals(emptyList<Long>(), QueuePlan.toStart(list, rules, online))
        assertEquals(Wait.Wifi, QueuePlan.waitOf(rules, online))
        assertEquals(listOf(1L), QueuePlan.toStart(list, rules, wifi))
    }

    @Test
    fun nothing_starts_offline() {
        assertEquals(emptyList<Long>(), QueuePlan.toStart(listOf(queued(1)), QueueRules(), offline))
        assertEquals(Wait.Network, QueuePlan.waitOf(QueueRules(), offline))
    }

    @Test
    fun a_scheduled_queue_waits_until_it_opens() {
        val list = listOf(queued(1))
        val closed = QueueRules(scheduled = true, open = false)
        assertEquals(emptyList<Long>(), QueuePlan.toStart(list, closed, online))
        assertEquals(Wait.Schedule, QueuePlan.waitOf(closed, online))
        assertEquals(listOf(1L), QueuePlan.toStart(list, closed.copy(open = true), online))
    }

    @Test
    fun a_limit_below_one_still_runs_one() {
        assertEquals(listOf(1L), QueuePlan.toStart(listOf(queued(1), queued(2)), QueueRules(maxRunning = 0), online))
    }

    @Test
    fun each_queue_starts_its_own_by_its_own_rules() {
        val night = Queue(1, "Night", QueueRules(maxRunning = 1, wifiOnly = true))
        val queues = listOf(Queue(Queue.MAIN, "", QueueRules(maxRunning = 2)), night)
        val list = listOf(queued(1), queued(2), queued(3), queued(4).copy(queue = 1), queued(5).copy(queue = 1))
        // On mobile data: the main queue's two; the night queue waits for Wi-Fi.
        assertEquals(listOf(1L, 2L), QueuePlan.toStart(list, queues, NetworkState(connected = true, unmetered = false)))
        // On Wi-Fi: the night queue's one too, its own limit.
        assertEquals(listOf(1L, 2L, 4L), QueuePlan.toStart(list, queues, NetworkState(connected = true, unmetered = true)))
    }

    @Test
    fun a_download_of_a_queue_that_is_gone_is_the_main_queues() {
        val queues = listOf(Queue(Queue.MAIN, "", QueueRules()))
        assertEquals(Queue.MAIN, QueuePlan.queueOf(queued(1).copy(queue = 7), queues))
        assertEquals(listOf(1L), QueuePlan.toStart(listOf(queued(1).copy(queue = 7)), queues, NetworkState(connected = true, unmetered = true)))
    }

    @Test
    fun moving_up_swaps_with_the_one_before() {
        val list = listOf(queued(1), running(2), queued(3), queued(4))
        val moved = QueuePlan.movedUp(list, 4)
        assertEquals(listOf(1L, 4L, 3L), QueuePlan.queued(moved).map { it.id })
        // The first in the queue stays first.
        assertEquals(list, QueuePlan.movedUp(list, 1))
    }

    @Test
    fun the_next_start_is_today_if_still_ahead_and_tomorrow_if_passed() {
        val zone = ZoneId.of("Africa/Cairo")
        val now = ZonedDateTime.of(2026, 10, 31, 22, 15, 0, 0, zone)
        assertEquals(now.withHour(23).withMinute(0), QueuePlan.nextStart(now, LocalTime.of(23, 0)))
        assertEquals(
            ZonedDateTime.of(2026, 11, 1, 2, 0, 0, 0, zone),
            QueuePlan.nextStart(now, LocalTime.of(2, 0)),
        )
        // Exactly now is not ahead: tomorrow.
        assertEquals(now.plusDays(1), QueuePlan.nextStart(now, LocalTime.of(22, 15)))
    }
}
