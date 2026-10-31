package io.github.motalim_ai.zajil.downloads

import android.content.Context
import java.time.LocalTime
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * The queue's rules, in the app's own preferences: a few values that only
 * this app reads, private to it like the download list (ADR-0012).
 */
class QueueSettings(context: Context) {
    private val prefs = context.getSharedPreferences("queue", Context.MODE_PRIVATE)
    private val _rules = MutableStateFlow(read())
    val rules: StateFlow<QueueRules> = _rules.asStateFlow()

    fun update(how: (QueueRules) -> QueueRules) {
        synchronized(this) {
            val rules = how(_rules.value)
            prefs.edit()
                .putInt(MAX_RUNNING, rules.maxRunning)
                .putBoolean(WIFI_ONLY, rules.wifiOnly)
                .putBoolean(SCHEDULED, rules.scheduled)
                .putInt(START_AT, rules.startAt.toSecondOfDay() / 60)
                .putBoolean(OPEN, rules.open)
                .apply()
            _rules.value = rules
        }
    }

    private fun read() = QueueRules(
        maxRunning = prefs.getInt(MAX_RUNNING, 3).coerceIn(1, MAX),
        wifiOnly = prefs.getBoolean(WIFI_ONLY, false),
        scheduled = prefs.getBoolean(SCHEDULED, false),
        startAt = LocalTime.ofSecondOfDay(prefs.getInt(START_AT, 120).coerceIn(0, 24 * 60 - 1) * 60L),
        open = prefs.getBoolean(OPEN, false),
    )

    companion object {
        /** The most downloads at once the screen offers. */
        const val MAX = 8
        private const val MAX_RUNNING = "max_running"
        private const val WIFI_ONLY = "wifi_only"
        private const val SCHEDULED = "scheduled"
        private const val START_AT = "start_at_minute"
        private const val OPEN = "open"
    }
}
