package io.github.motalim_ai.zajil.downloads

import io.github.motalim_ai.zajil.engine.Problem
import org.json.JSONArray
import org.json.JSONObject

enum class Status {
    /** Waits in the queue: for its turn, the schedule, or the network (A2). */
    Queued,

    /** The engine is downloading it. */
    Running,

    /** Was running when the app or the phone stopped; resumes by itself. */
    Interrupted,

    /** Paused by the user. */
    Paused,

    /**
     * Its link is on the local network, and Android 17 lets Zajil there only
     * with the user's permission, which it does not have (yet).
     */
    NeedsLocalNetwork,

    /** Stopped by a problem; resumable if it has a state file. */
    Failed,

    /** A video's streams are complete; MediaMuxer joins them (A3). */
    Merging,

    /** Complete; being moved to the public Download folder. */
    Publishing,

    /** Complete, in the public Download folder. */
    Done,
}

/**
 * One download, as the app keeps it between runs. The bytes themselves and
 * where they stopped are in the engine's own files ([file].zajil-part and
 * [stateFile], ADR-0004); this only remembers which download they belong to.
 */
data class Download(
    val id: Long,
    val url: String,
    val status: Status,
    /** The file the engine writes, once known (its final path). */
    val file: String? = null,
    /** Resumes it, once data has started to flow. */
    val stateFile: String? = null,
    val done: Long = 0,
    val total: Long? = null,
    val problem: Problem? = null,
    val detail: String? = null,
    /** Where the finished file is: a content URI of the public Download folder. */
    val uri: String? = null,
    /** The finished file's name there, which may differ from [file]'s. */
    val shownName: String? = null,
    /** Its link points to the local network ([LocalNetwork]). */
    val local: Boolean = false,
    /** Its place in the queue: the smaller, the sooner. */
    val order: Long = id,
    /** A video read by yt-dlp: its streams (A3). [url] is then the page's link. */
    val video: VideoJob? = null,
    /** The name the user gave it before it started, instead of the server's. */
    val chosenName: String? = null,
    /** When it was complete (milliseconds since 1970). */
    val finishedAt: Long? = null,
    /** The in-app browser's page it was found on (A6). */
    val page: String? = null,
    /** Whose cookies go with it; the cookies themselves are never kept here. */
    val cookieSource: CookieSource = CookieSource.None,
    /** The in-app browser's User-Agent, when it came from there. */
    val browserAgent: String? = null,
) {
    /**
     * Before the engine knows the file's name: the link's last path part,
     * never its query, which may hold a token (AGENTS.md, section 10.3).
     */
    val name: String
        get() = shownName ?: video?.title ?: file?.substringAfterLast('/') ?: chosenName ?: shortLink(url)

    fun toJson(): JSONObject = JSONObject().apply {
        put("id", id)
        put("url", url)
        put("status", status.name)
        put("file", file)
        put("state_file", stateFile)
        put("done", done)
        put("total", total)
        put("problem", problem?.name)
        put("detail", detail)
        put("uri", uri)
        put("shown_name", shownName)
        put("local", local)
        put("order", order)
        put("video", video?.toJson())
        put("chosen_name", chosenName)
        put("finished_at", finishedAt)
        put("page", page)
        put("cookie_source", cookieSource.name)
        put("browser_agent", browserAgent)
    }

    companion object {
        fun fromJson(json: JSONObject) = Download(
            id = json.getLong("id"),
            url = json.getString("url"),
            status = Status.valueOf(json.getString("status")),
            file = json.optStringOrNull("file"),
            stateFile = json.optStringOrNull("state_file"),
            done = json.optLong("done"),
            total = if (json.isNull("total")) null else json.getLong("total"),
            problem = json.optStringOrNull("problem")?.let { name ->
                Problem.entries.firstOrNull { it.name == name }
            },
            detail = json.optStringOrNull("detail"),
            uri = json.optStringOrNull("uri"),
            shownName = json.optStringOrNull("shown_name"),
            local = json.optBoolean("local"),
            order = json.optLong("order", json.getLong("id")),
            video = json.optJSONObject("video")?.let(VideoJob::fromJson),
            chosenName = json.optStringOrNull("chosen_name"),
            finishedAt = if (json.isNull("finished_at")) null else json.optLong("finished_at"),
            page = json.optStringOrNull("page"),
            cookieSource = CookieSource.entries.firstOrNull { it.name == json.optStringOrNull("cookie_source") } ?: CookieSource.None,
            browserAgent = json.optStringOrNull("browser_agent"),
        )

        fun listToJson(downloads: List<Download>): String =
            JSONArray(downloads.map(Download::toJson)).toString()

        fun listFromJson(text: String): List<Download> = JSONArray(text).let { array ->
            (0 until array.length()).map { fromJson(array.getJSONObject(it)) }
        }
    }
}

/** The link's last path part, decoded («الحلقة 12.mp4»), else its host: never its query. */
private fun shortLink(url: String): String {
    val uri = android.net.Uri.parse(url)
    val host = uri.host ?: return url.substringBefore('?').substringBefore('#')
    return uri.lastPathSegment?.takeIf { it.isNotBlank() } ?: host
}

private fun JSONObject.optStringOrNull(key: String): String? =
    if (isNull(key)) null else getString(key)
