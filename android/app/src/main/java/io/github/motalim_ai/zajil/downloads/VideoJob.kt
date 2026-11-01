package io.github.motalim_ai.zajil.downloads

import io.github.motalim_ai.zajil.video.Container
import org.json.JSONArray
import org.json.JSONObject

/**
 * One stream of a video: the picture or the sound, which Zajil's engine
 * downloads to its own file, as on the desktop (A3).
 */
data class Stream(
    val formatId: String,
    val url: String,
    /** Its file's name in the download's folder ("video.mp4", "audio.m4a"). */
    val name: String,
    val userAgent: String?,
    val video: Boolean,
    val file: String? = null,
    val stateFile: String? = null,
    val done: Long = 0,
    val total: Long? = null,
    val finished: Boolean = false,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("format_id", formatId)
        put("url", url)
        put("name", name)
        put("user_agent", userAgent)
        put("video", video)
        put("file", file)
        put("state_file", stateFile)
        put("done", done)
        put("total", total)
        put("finished", finished)
    }

    companion object {
        fun fromJson(o: JSONObject) = Stream(
            formatId = o.getString("format_id"),
            url = o.getString("url"),
            name = o.getString("name"),
            userAgent = o.opt("user_agent") as? String,
            video = o.optBoolean("video"),
            file = o.opt("file") as? String,
            stateFile = o.opt("state_file") as? String,
            done = o.optLong("done"),
            total = if (o.isNull("total")) null else o.getLong("total"),
            finished = o.optBoolean("finished"),
        )
    }
}

/** A video download: its title, the streams it is made of, and what they make. */
data class VideoJob(
    val title: String,
    val container: Container,
    val streams: List<Stream>,
    /** What the user chose, for the screen ("1080p · الصوت الأصلي"). */
    val choice: String,
    /** yt-dlp asked again for fresh links after a refusal, once per run. */
    val refreshed: Boolean = false,
    /** An HLS playlist: yt-dlp downloads its one stream, and MediaMuxer makes it MP4 (A6). */
    val hls: Boolean = false,
    /** DASH (A6): yt-dlp downloads each stream in turn, and MediaMuxer joins them. */
    val dash: Boolean = false,
) {
    /** Downloaded by yt-dlp, not by Zajil's engine. */
    val byYtDlp get() = hls || dash

    fun toJson(): JSONObject = JSONObject().apply {
        put("title", title)
        put("container", container.name)
        put("streams", JSONArray(streams.map(Stream::toJson)))
        put("choice", choice)
        put("refreshed", refreshed)
        put("hls", hls)
        put("dash", dash)
    }

    companion object {
        fun fromJson(o: JSONObject) = VideoJob(
            title = o.getString("title"),
            container = Container.valueOf(o.getString("container")),
            streams = o.getJSONArray("streams").let { a -> (0 until a.length()).map { Stream.fromJson(a.getJSONObject(it)) } },
            choice = o.optString("choice"),
            refreshed = o.optBoolean("refreshed"),
            hls = o.optBoolean("hls"),
            dash = o.optBoolean("dash"),
        )
    }
}

/**
 * A playlist's video before its turn (0.3.0, as the desktop's playlist
 * rows): its page is read when it starts, and the quality and sound are
 * picked then, the nearest the page has to these.
 */
data class PlaylistPlan(
    val title: String,
    /** The highest picture wanted; null: the sound only. */
    val height: Int?,
    /** A dubbed sound's language, if the page has it; null: the video's own. */
    val dub: String?,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("title", title)
        put("height", height)
        put("dub", dub)
    }

    companion object {
        fun fromJson(o: JSONObject) = PlaylistPlan(
            title = o.optString("title"),
            height = if (o.isNull("height")) null else o.optInt("height"),
            dub = if (o.isNull("dub")) null else o.optString("dub"),
        )
    }
}
