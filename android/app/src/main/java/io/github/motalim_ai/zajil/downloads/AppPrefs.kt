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
    /**
     * The folder the user chose with Android's folder picker (a document
     * tree URI, its permission kept); null: Download/Zajil.
     */
    val saveTree: String? = null,
    /** A folder for each type, as the desktop's categories ([SaveCategory]). */
    val folderPerType: Boolean = false,
    /** The video quality the video sheet chooses first: the highest at or under it (D5's «الجودة المفضلة»). */
    val videoHeight: Int = DEFAULT_VIDEO_HEIGHT,
) {
    companion object {
        const val DEFAULT_CONNECTIONS = 8

        /** As the video sheet chose before it could be set. */
        const val DEFAULT_VIDEO_HEIGHT = 1080

        /** The qualities the screen offers, in lines. */
        val VIDEO_HEIGHTS = listOf(2160, 1440, 1080, 720, 480, 360)

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
                .putString(SAVE_TREE, prefs.saveTree)
                .putBoolean(FOLDER_PER_TYPE, prefs.folderPerType)
                .putInt(VIDEO_HEIGHT, prefs.videoHeight)
                .apply()
            _prefs.value = prefs
        }
    }

    private fun read() = Prefs(
        look = Look.entries.firstOrNull { it.name == store.getString(LOOK, null) } ?: Look.System,
        connections = store.getInt(CONNECTIONS, Prefs.DEFAULT_CONNECTIONS).coerceIn(1, 16),
        speedLimit = store.getLong(SPEED_LIMIT, 0).takeIf { it > 0 },
        saveTree = store.getString(SAVE_TREE, null),
        folderPerType = store.getBoolean(FOLDER_PER_TYPE, false),
        videoHeight = store.getInt(VIDEO_HEIGHT, Prefs.DEFAULT_VIDEO_HEIGHT).coerceIn(144, 4320),
    )

    private companion object {
        const val LOOK = "look"
        const val CONNECTIONS = "connections"
        const val SPEED_LIMIT = "speed_limit"
        const val SAVE_TREE = "save_tree"
        const val FOLDER_PER_TYPE = "folder_per_type"
        const val VIDEO_HEIGHT = "video_height"
    }
}
