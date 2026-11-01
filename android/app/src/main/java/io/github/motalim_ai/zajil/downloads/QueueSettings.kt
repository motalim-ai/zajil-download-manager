package io.github.motalim_ai.zajil.downloads

import android.content.Context
import java.time.DayOfWeek
import java.time.LocalTime
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONArray
import org.json.JSONObject

/**
 * The queues and their rules, in the app's own preferences: a few values
 * that only this app reads, private to it like the download list
 * (ADR-0012). The main queue is always there, first; before 0.3.0 its rules
 * were the only ones, under the keys it still starts from.
 */
class QueueSettings(context: Context) {
    private val prefs = context.getSharedPreferences("queue", Context.MODE_PRIVATE)
    private val _queues = MutableStateFlow(read())
    val queues: StateFlow<List<Queue>> = _queues.asStateFlow()

    /** [id]'s queue, or the main one if it is gone. */
    fun queue(id: Long): Queue = _queues.value.firstOrNull { it.id == id } ?: _queues.value.first()

    fun update(id: Long, how: (QueueRules) -> QueueRules) = save { list ->
        list.map { if (it.id == id) it.copy(rules = how(it.rules)) else it }
    }

    /** A new queue named [name], with the main queue's defaults; its id. */
    fun create(name: String): Long {
        var id = 0L
        save { list ->
            id = (list.maxOf { it.id }) + 1
            list + Queue(id, name.trim().take(MAX_NAME), QueueRules())
        }
        return id
    }

    fun rename(id: Long, name: String) = save { list ->
        list.map { if (it.id == id && !it.main) it.copy(name = name.trim().take(MAX_NAME)) else it }
    }

    /** Deletes [id] (never the main queue); its downloads go to the main queue ([DownloadCenter.deleteQueue]). */
    fun delete(id: Long) = save { list -> list.filterNot { it.id == id && !it.main } }

    private fun save(how: (List<Queue>) -> List<Queue>) {
        synchronized(this) {
            val list = how(_queues.value)
            prefs.edit().putString(QUEUES, JSONArray(list.map(::toJson)).toString()).apply()
            _queues.value = list
        }
    }

    private fun read(): List<Queue> {
        val stored = prefs.getString(QUEUES, null)?.let { text ->
            runCatching {
                JSONArray(text).let { a -> (0 until a.length()).map { fromJson(a.getJSONObject(it)) } }
            }.getOrNull()
        }
        if (!stored.isNullOrEmpty() && stored.any { it.main }) return stored.sortedBy { if (it.main) 0 else 1 }
        // Before 0.3.0: one queue, its rules under their own keys.
        return listOf(
            Queue(
                Queue.MAIN,
                "",
                QueueRules(
                    maxRunning = prefs.getInt(MAX_RUNNING, 3).coerceIn(1, MAX),
                    wifiOnly = prefs.getBoolean(WIFI_ONLY, false),
                    scheduled = prefs.getBoolean(SCHEDULED, false),
                    startAt = LocalTime.ofSecondOfDay(prefs.getInt(START_AT, 120).coerceIn(0, 24 * 60 - 1) * 60L),
                    open = prefs.getBoolean(OPEN, false),
                ),
            ),
        )
    }

    private fun toJson(queue: Queue) = JSONObject()
        .put("id", queue.id)
        .put("name", queue.name)
        .put(MAX_RUNNING, queue.rules.maxRunning)
        .put(WIFI_ONLY, queue.rules.wifiOnly)
        .put(SCHEDULED, queue.rules.scheduled)
        .put(START_AT, queue.rules.startAt.toSecondOfDay() / 60)
        .put(OPEN, queue.rules.open)
        .put(DAYS, queue.rules.days.sumOf { 1 shl (it.value - 1) })

    private fun fromJson(o: JSONObject) = Queue(
        id = o.getLong("id"),
        name = o.optString("name").take(MAX_NAME),
        rules = QueueRules(
            maxRunning = o.optInt(MAX_RUNNING, 3).coerceIn(1, MAX),
            wifiOnly = o.optBoolean(WIFI_ONLY),
            scheduled = o.optBoolean(SCHEDULED),
            startAt = LocalTime.ofSecondOfDay(o.optInt(START_AT, 120).coerceIn(0, 24 * 60 - 1) * 60L),
            open = o.optBoolean(OPEN),
            days = daysOf(o.optInt(DAYS, ALL_DAYS)),
        ),
    )

    /** A bit for each day, Monday first; none counts as every day. */
    private fun daysOf(bits: Int): Set<DayOfWeek> =
        DayOfWeek.entries.filter { bits and (1 shl (it.value - 1)) != 0 }.toSet().ifEmpty { DayOfWeek.entries.toSet() }

    companion object {
        /** The most downloads at once the screen offers. */
        const val MAX = 8

        /** The longest queue name kept. */
        const val MAX_NAME = 40
        private const val QUEUES = "queues"
        private const val MAX_RUNNING = "max_running"
        private const val WIFI_ONLY = "wifi_only"
        private const val SCHEDULED = "scheduled"
        private const val START_AT = "start_at_minute"
        private const val OPEN = "open"
        private const val DAYS = "days"
        private const val ALL_DAYS = 0b1111111
    }
}
