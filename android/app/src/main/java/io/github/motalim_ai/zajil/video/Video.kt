package io.github.motalim_ai.zajil.video

import org.json.JSONObject

/** One format of a video, as yt-dlp found it (only the fields Zajil uses). */
data class Format(
    val id: String,
    val ext: String,
    val vcodec: String?,
    val acodec: String?,
    val height: Int?,
    val fps: Double?,
    val tbr: Double?,
    val size: Long?,
    val language: String?,
    val languagePreference: Int?,
    val note: String?,
    val url: String,
    val userAgent: String?,
    val hasDrm: Boolean,
) {
    val hasVideo get() = vcodec != null && vcodec != "none"
    val hasAudio get() = acodec != null && acodec != "none"

    /**
     * How much its sound is the video's own, as on the desktop
     * (`zajil-core` video.rs, `sound_rank`): yt-dlp marks the original
     * «original» and ranks it higher; a dubbed one lower.
     */
    val soundRank: Int
        get() {
            val note = note.orEmpty().lowercase()
            val marked = when {
                "original" in note -> 10
                "dubbed" in note -> -10
                else -> 0
            }
            return marked + (languagePreference ?: 0)
        }
}

/** A video page that yt-dlp read. */
data class VideoInfo(
    val id: String,
    val title: String,
    val duration: Double?,
    val live: Boolean,
    val formats: List<Format>,
    val ytDlpVersion: String,
    /** The picture the page shows for it (an https link), for the format sheet. */
    val thumbnail: String? = null,
    /** Its channel's name. */
    val channel: String? = null,
) {
    companion object {
        /** yt-dlp's answer; an [error] if it could not read the page. */
        fun parse(json: String): Result<VideoInfo> {
            val o = JSONObject(json)
            if (o.has("error")) return Result.failure(VideoError(o.getString("error")))
            val formats = o.getJSONArray("formats").let { array ->
                (0 until array.length()).map { i ->
                    val f = array.getJSONObject(i)
                    Format(
                        id = f.getString("format_id"),
                        ext = f.optString("ext"),
                        vcodec = f.str("vcodec"),
                        acodec = f.str("acodec"),
                        height = f.int("height"),
                        fps = f.dbl("fps"),
                        tbr = f.dbl("tbr"),
                        size = f.long("filesize") ?: f.long("filesize_approx"),
                        language = f.str("language"),
                        languagePreference = f.int("language_preference"),
                        note = f.str("format_note"),
                        url = f.getString("url"),
                        userAgent = f.optJSONObject("http_headers")?.str("User-Agent"),
                        hasDrm = f.optBoolean("has_drm"),
                    )
                }
            }
            return Result.success(
                VideoInfo(
                    id = o.str("id").orEmpty(),
                    title = o.str("title") ?: o.str("id").orEmpty(),
                    duration = o.dbl("duration"),
                    live = o.optBoolean("is_live") || o.str("live_status") == "is_live",
                    formats = formats,
                    ytDlpVersion = o.str("version").orEmpty(),
                    thumbnail = o.str("thumbnail")?.takeIf { it.startsWith("https://") },
                    channel = o.str("channel") ?: o.str("uploader"),
                ),
            )
        }

        private fun JSONObject.str(key: String) = if (isNull(key)) null else optString(key).takeIf { it.isNotEmpty() }
        private fun JSONObject.int(key: String) = if (isNull(key)) null else optDouble(key).takeIf { !it.isNaN() }?.toInt()
        private fun JSONObject.long(key: String) = if (isNull(key)) null else optDouble(key).takeIf { !it.isNaN() }?.toLong()
        private fun JSONObject.dbl(key: String) = if (isNull(key)) null else optDouble(key).takeIf { !it.isNaN() }
    }
}

class VideoError(message: String) : Exception(message)

/** A sound to offer: the video's own, or one dubbed into a language. */
data class Sound(val language: String?, val original: Boolean)

/**
 * The file Zajil makes: a container MediaMuxer writes (ADR-0012, section
 * 6), with no conversion of the picture or the sound.
 */
enum class Container(val ext: String, val mime: String) {
    Mp4("mp4", "video/mp4"),
    Webm("webm", "video/webm"),
    M4a("m4a", "audio/mp4"),
}

/** What to download for a choice: the picture (none for sound only) and the sound. */
data class Picked(val video: Format?, val audio: Format, val container: Container) {
    val size: Long? get() = audio.size?.let { a -> if (video == null) a else video.size?.plus(a) }
}

/**
 * Which formats make the file the user chose: the same idea as the
 * desktop's choice of quality and sound, without FFmpeg. MediaMuxer
 * writes H.264 with AAC into MP4, and VP9 with Opus into WebM; AV1 and
 * the rest are left out, so nothing ever needs converting.
 */
object VideoChoice {
    private fun muxable(f: Format): Container? = when {
        !f.hasVideo || f.hasAudio || f.hasDrm -> null
        f.vcodec!!.startsWith("avc1") -> Container.Mp4
        f.vcodec.startsWith("vp9") || f.vcodec.startsWith("vp09") -> Container.Webm
        else -> null
    }

    private fun fits(f: Format, container: Container): Boolean = when {
        !f.hasAudio || f.hasVideo || f.hasDrm -> false
        container == Container.Webm -> f.acodec!!.startsWith("opus")
        else -> f.acodec!!.startsWith("mp4a")
    }

    /** The heights offered, highest first. */
    fun qualities(info: VideoInfo): List<Int> =
        info.formats.filter { muxable(it) != null }.mapNotNull { it.height }.distinct().sortedDescending()

    /** The sounds offered: the video's own first, then the dubbed ones by language. */
    fun sounds(info: VideoInfo): List<Sound> {
        val audios = info.formats.filter { it.hasAudio && !it.hasVideo && !it.hasDrm }
        if (audios.isEmpty()) return emptyList()
        val top = audios.maxOf { it.soundRank }
        return audios
            .map { Sound(it.language, original = it.soundRank == top) }
            .distinct()
            .sortedWith(compareByDescending<Sound> { it.original }.thenBy { it.language.orEmpty() })
    }

    /**
     * The formats for [height] (null: the sound alone) and [sound]: the
     * best picture of that height that has a sound in [sound]'s language
     * to go with it in one container. Null if there is none.
     */
    fun pick(info: VideoInfo, height: Int?, sound: Sound): Picked? {
        val audios = info.formats.filter { it.hasAudio && !it.hasVideo && it.matches(sound, info) }
        if (height == null) {
            val audio = audios.filter { fits(it, Container.M4a) }.maxWithOrNull(bestSound) ?: return null
            return Picked(null, audio, Container.M4a)
        }
        val videos = info.formats.filter { it.height == height && muxable(it) != null }
            // H.264 in MP4 first: every player plays it.
            .sortedWith(compareBy<Format> { if (muxable(it) == Container.Mp4) 0 else 1 }.thenByDescending { it.tbr ?: 0.0 })
        for (video in videos) {
            val container = muxable(video)!!
            val audio = audios.filter { fits(it, container) }.maxWithOrNull(bestSound) ?: continue
            return Picked(video, audio, container)
        }
        return null
    }

    /**
     * The plain sound before YouTube's range-compressed one («drc», quieter
     * peaks for small speakers), then the higher rate.
     */
    private val bestSound = compareBy<Format> { if ("drc" in it.id) 0 else 1 }.thenBy { it.tbr ?: 0.0 }

    private fun Format.matches(sound: Sound, info: VideoInfo): Boolean {
        val top = info.formats.filter { it.hasAudio && !it.hasVideo }.maxOfOrNull { it.soundRank }
        return if (sound.original) soundRank == top else soundRank != top && language == sound.language
    }
}

/** Links Zajil reads with yt-dlp rather than downloading the page itself. */
object VideoPages {
    private val hosts = setOf("youtube.com", "m.youtube.com", "music.youtube.com", "youtu.be", "youtube-nocookie.com")

    fun isVideoPage(url: String): Boolean {
        val uri = runCatching { java.net.URI(url) }.getOrNull() ?: return false
        val host = uri.host?.lowercase()?.removePrefix("www.") ?: return false
        if (host !in hosts) return false
        val path = uri.path.orEmpty()
        return host == "youtu.be" || path.startsWith("/watch") || path.startsWith("/shorts/") ||
            path.startsWith("/live/") || path.startsWith("/embed/")
    }

    /**
     * Whether a YouTube video server's link is about to expire, by its own
     * `expire=` (seconds since 1970): with ten minutes' margin, as on the
     * desktop, so that a resumed stream asks yt-dlp for a fresh link first.
     */
    fun expiresSoon(url: String, nowSeconds: Long = System.currentTimeMillis() / 1000): Boolean {
        val expire = Regex("[?&]expire=(\\d+)").find(url)?.groupValues?.get(1)?.toLongOrNull() ?: return false
        return expire - nowSeconds < 10 * 60
    }
}
