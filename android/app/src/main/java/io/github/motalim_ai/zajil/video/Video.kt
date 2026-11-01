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
    /** An HLS playlist: yt-dlp downloads it itself (A6). */
    val hls: Boolean = false,
    /**
     * The cookies its address needs, which yt-dlp got while reading the
     * page (TikTok's): in memory only, sent to that address only.
     */
    val cookieHeader: String? = null,
) {
    val hasVideo get() = vcodec != null && vcodec != "none"
    val hasAudio get() = acodec != null && acodec != "none"

    /**
     * One file with the picture and the sound (X's, Facebook's, TikTok's
     * MP4): codecs both said, or neither (the site did not say). Played as
     * it is, nothing to join.
     */
    val isWhole get() = !hasDrm && ((hasVideo && hasAudio) || (vcodec == null && acodec == null))

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
                        hls = f.optBoolean("hls"),
                        cookieHeader = f.str("cookie_header"),
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

/**
 * What to download for a choice: the picture (none for sound only) and the
 * sound; or one whole file ([audio] null: [video] has both, or an HLS
 * playlist yt-dlp downloads).
 */
data class Picked(val video: Format?, val audio: Format?, val container: Container) {
    val size: Long? get() = listOfNotNull(video, audio).map { it.size }.let { sizes ->
        if (sizes.isEmpty() || sizes.any { it == null }) null else sizes.sumOf { it!! }
    }

    /** One file, nothing to join. */
    val whole get() = audio == null && video != null
}

/**
 * Which formats make the file the user chose: the same idea as the
 * desktop's choice of quality and sound, without FFmpeg. MediaMuxer
 * writes H.264 with AAC into MP4, and VP9 with Opus into WebM; AV1 and
 * the rest are left out, so nothing ever needs converting.
 */
object VideoChoice {
    private fun muxable(f: Format): Container? = when {
        !f.hasVideo || f.hasAudio || f.hasDrm || f.hls -> null
        f.vcodec!!.startsWith("avc1") -> Container.Mp4
        f.vcodec.startsWith("vp9") || f.vcodec.startsWith("vp09") -> Container.Webm
        else -> null
    }

    private fun fits(f: Format, container: Container): Boolean = when {
        !f.hasAudio || f.hasVideo || f.hasDrm || f.hls -> false
        container == Container.Webm -> f.acodec!!.startsWith("opus")
        else -> f.acodec!!.startsWith("mp4a")
    }

    /** Whole files (picture and sound in one), not HLS. */
    private fun wholes(info: VideoInfo) = info.formats.filter { !it.hls && it.isWhole }

    private fun playlists(info: VideoInfo) = info.formats.filter { it.hls && !it.hasDrm }

    /**
     * The heights offered, highest first: of pictures to join with a
     * sound, of whole files, and of HLS playlists. [WHOLE] is a page's one
     * whole file whose height the site does not say (Facebook's «hd»).
     */
    fun qualities(info: VideoInfo): List<Int> {
        // A picture alone counts only with a sound to join it to (Facebook
        // gives VP9 pictures with no sound beside its whole files).
        val pictures = if (hasSoundOnly(info)) info.formats.filter { muxable(it) != null } else emptyList()
        val heights = (pictures + wholes(info) + playlists(info))
            .mapNotNull { it.height }.filter { it > 0 }.distinct().sortedDescending()
        if (heights.isNotEmpty()) return heights
        return if (wholes(info).isNotEmpty() || playlists(info).isNotEmpty()) listOf(WHOLE) else emptyList()
    }

    /** «Sound only» can be offered: the page has a sound alone. */
    fun hasSoundOnly(info: VideoInfo) = info.formats.any { it.hasAudio && !it.hasVideo && !it.hasDrm && !it.hls }

    /**
     * The sounds offered: the video's own first, then the dubbed ones by
     * language. A page of whole files only has its own sound.
     */
    fun sounds(info: VideoInfo): List<Sound> {
        val audios = info.formats.filter { it.hasAudio && !it.hasVideo && !it.hasDrm && !it.hls }
        if (audios.isEmpty()) {
            return if (wholes(info).isNotEmpty() || playlists(info).isNotEmpty()) listOf(Sound(null, original = true)) else emptyList()
        }
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
        val audios = info.formats.filter { it.hasAudio && !it.hasVideo && !it.hls && it.matches(sound, info) }
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
        // A whole file of that height (or the page's best one, [WHOLE]):
        // H.264 in MP4 first, then the larger.
        if (sound.original) {
            // yt-dlp lists formats worst first: among equals, the last («hd» after «sd»).
            val whole = wholes(info).filter { if (height == WHOLE) true else it.height == height }
                .sortedWith(compareBy<Format> { if (it.ext == "mp4") 1 else 0 }.thenBy { it.height ?: 0 }.thenBy { it.size ?: 0 }.thenBy { it.tbr ?: 0.0 })
                .lastOrNull()
            if (whole != null) return Picked(whole, null, containerOf(whole.ext))
            val playlist = playlists(info).filter { if (height == WHOLE) true else it.height == height }
                .sortedWith(compareBy<Format> { it.height ?: 0 }.thenBy { it.tbr ?: 0.0 })
                .lastOrNull()
            // HLS is saved as MP4 (MediaMuxer, from its MPEG-TS).
            if (playlist != null) return Picked(playlist, null, Container.Mp4)
        }
        return null
    }

    /** The height of a page's whole file the site gives no height for. */
    const val WHOLE = 0

    private fun containerOf(ext: String) = when (ext) {
        "webm" -> Container.Webm
        "m4a" -> Container.M4a
        else -> Container.Mp4
    }

    /**
     * The plain sound before YouTube's range-compressed one («drc», quieter
     * peaks for small speakers), then the higher rate.
     */
    private val bestSound = compareBy<Format> { if ("drc" in it.id) 0 else 1 }.thenBy { it.tbr ?: 0.0 }

    private fun Format.matches(sound: Sound, info: VideoInfo): Boolean {
        val top = info.formats.filter { it.hasAudio && !it.hasVideo && !it.hls }.maxOfOrNull { it.soundRank }
        return if (sound.original) soundRank == top else soundRank != top && language == sound.language
    }
}

/**
 * Links Zajil reads with yt-dlp rather than downloading the page itself:
 * the desktop's list (apps/desktop/src/lib/videoSites.ts), the big sites'
 * video pages by their own paths.
 */
object VideoPages {
    private val pages: List<Pair<Regex, Regex>> = listOf(
        Regex("^(www\\.|m\\.|music\\.)?youtube\\.com$") to Regex("^/(watch|shorts/|live/|embed/)"),
        Regex("^(www\\.)?youtube-nocookie\\.com$") to Regex("^/embed/"),
        Regex("^youtu\\.be$") to Regex("^/[\\w-]+"),
        Regex("^(www\\.|mobile\\.)?(x|twitter)\\.com$") to Regex("^/[^/]+/status/\\d+"),
        Regex("^t\\.co$") to Regex("^/[\\w-]+"),
        Regex("^(www\\.)?instagram\\.com$") to Regex("^/(p|reel|reels|tv)/"),
        Regex("^(www\\.|m\\.|web\\.)?facebook\\.com$") to Regex("^/(.+/videos/|watch|reel/|share/(v|r)/)"),
        Regex("^fb\\.watch$") to Regex("^/[\\w-]+"),
        Regex("^(www\\.|m\\.)?tiktok\\.com$") to Regex("^/@[^/]+/video/\\d+"),
        Regex("^(vm|vt)\\.tiktok\\.com$") to Regex("^/[\\w-]+"),
        Regex("^(www\\.)?vimeo\\.com$") to Regex("^/\\d+"),
        Regex("^(www\\.)?dailymotion\\.com$") to Regex("^/video/"),
    )

    fun isVideoPage(url: String): Boolean {
        val uri = runCatching { java.net.URI(url.trim()) }.getOrNull() ?: return false
        if (uri.scheme?.lowercase() !in setOf("http", "https")) return false
        val host = uri.host?.lowercase() ?: return false
        val path = uri.rawPath.orEmpty()
        return pages.any { (h, p) -> h.containsMatchIn(host) && p.containsMatchIn(path) }
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
