package io.github.motalim_ai.zajil.downloads

import android.content.Context
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Light, dark, or as the system says (the desktop's «Appearance»). */
enum class Look { System, Light, Dark }

/** The app's settings beyond the queue's ([QueueSettings]). */
data class Prefs(
    val look: Look = Look.System,
    /** Connections per file, as the desktop's default (`zajil-core` segmented.rs). */
    val connections: Int = DEFAULT_CONNECTIONS,
    /** All downloads together, in bytes per second; null: no limit. */
    val speedLimit: Long? = null,
) {
    companion object {
        const val DEFAULT_CONNECTIONS = 8

        /** The connections the screen offers. */
        val CONNECTIONS = listOf(1, 2, 4, 8, 16)

        /** The limits the screen offers, in bytes per second. */
        val SPEED_LIMITS: List<Long?> = listOf(null, 256L shl 10, 512L shl 10, 1L shl 20, 2L shl 20, 5L shl 20, 10L shl 20)
    }
}

/** [Prefs] in the app's own preferences, private to it like the download list. */
class AppPrefs(context: Context) {
    private val store = context.getSharedPreferences("app", Context.MODE_PRIVATE)
    private val _prefs = MutableStateFlow(read())
    val prefs: StateFlow<Prefs> = _prefs.asStateFlow()

    fun update(how: (Prefs) -> Prefs) {
        synchronized(this) {
            val prefs = how(_prefs.value)
            store.edit()
                .putString(LOOK, prefs.look.name)
                .putInt(CONNECTIONS, prefs.connections)
                .putLong(SPEED_LIMIT, prefs.speedLimit ?: 0)
                .apply()
            _prefs.value = prefs
        }
    }

    private fun read() = Prefs(
        look = Look.entries.firstOrNull { it.name == store.getString(LOOK, null) } ?: Look.System,
        connections = store.getInt(CONNECTIONS, Prefs.DEFAULT_CONNECTIONS).coerceIn(1, 16),
        speedLimit = store.getLong(SPEED_LIMIT, 0).takeIf { it > 0 },
    )

    private companion object {
        const val LOOK = "look"
        const val CONNECTIONS = "connections"
        const val SPEED_LIMIT = "speed_limit"
    }
}
