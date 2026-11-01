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
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("title", title)
        put("container", container.name)
        put("streams", JSONArray(streams.map(Stream::toJson)))
        put("choice", choice)
        put("refreshed", refreshed)
        put("hls", hls)
    }

    companion object {
        fun fromJson(o: JSONObject) = VideoJob(
            title = o.getString("title"),
            container = Container.valueOf(o.getString("container")),
            streams = o.getJSONArray("streams").let { a -> (0 until a.length()).map { Stream.fromJson(a.getJSONObject(it)) } },
            choice = o.optString("choice"),
            refreshed = o.optBoolean("refreshed"),
            hls = o.optBoolean("hls"),
        )
    }
}
