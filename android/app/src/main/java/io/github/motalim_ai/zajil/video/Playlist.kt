package io.github.motalim_ai.zajil.video

import org.json.JSONObject

/** One video of a playlist as yt-dlp listed it, not read yet. Everything in it came from the site. */
data class PlaylistEntry(
    /** Its page (http(s)); null if the site gave none. */
    val url: String?,
    val title: String?,
    val duration: Double?,
    /** A small picture (https). */
    val thumbnail: String?,
    /** A playlist itself (a channel's tab): opened, never downloaded as one. */
    val playlist: Boolean,
    /** Not private, removed or for members only, as far as the list says. */
    val available: Boolean,
) {
    /** Can be checked and downloaded. */
    val downloadable get() = url != null && !playlist && available
}

/** A playlist or a channel's videos (0.3.0; the desktop's since 1.1): its title and entries. */
data class PlaylistInfo(val title: String, val channel: String?, val entries: List<PlaylistEntry>)

/** What a page holds, as yt-dlp read it. */
sealed interface Page {
    data class Video(val info: VideoInfo) : Page

    data class Playlist(val info: PlaylistInfo) : Page

    companion object {
        /** The most entries listed, as the desktop's `MAX_PLAYLIST_ENTRIES`. */
        const val MAX_ENTRIES = 1000

        fun parse(json: String): Result<Page> {
            val o = runCatching { JSONObject(json) }.getOrElse { return Result.failure(it) }
            if (!o.optBoolean("playlist")) return VideoInfo.parse(json).map(::Video)
            val array = o.optJSONArray("entries")
            val entries = (0 until minOf(array?.length() ?: 0, MAX_ENTRIES)).mapNotNull { i ->
                val e = array!!.optJSONObject(i) ?: return@mapNotNull null
                PlaylistEntry(
                    url = e.text("url")?.takeIf { it.startsWith("https://") || it.startsWith("http://") },
                    title = e.text("title")?.take(300),
                    duration = if (e.isNull("duration")) null else e.optDouble("duration").takeIf { it.isFinite() && it >= 0 },
                    thumbnail = e.text("thumbnail")?.takeIf { it.startsWith("https://") },
                    playlist = e.optBoolean("playlist"),
                    available = e.optBoolean("available", true),
                )
            }
            return Result.success(Playlist(PlaylistInfo(o.text("title").orEmpty().take(300), o.text("channel")?.take(200), entries)))
        }

        private fun JSONObject.text(key: String): String? = if (isNull(key)) null else optString(key).takeIf { it.isNotEmpty() }
    }
}

/**
 * Links that are a video in a list (YouTube's `watch?v=…&list=…`): the
 * user says whether they want the video or the whole list.
 */
object PlaylistLinks {
    /** A playlist's or a channel's own page (YouTube's), or a video in a list. */
    fun isList(url: String): Boolean {
        if (listOf(url) != null) return true
        val uri = runCatching { java.net.URI(url.trim()) }.getOrNull() ?: return false
        if (!Regex("^(www\\.|m\\.|music\\.)?youtube\\.com$").matches(uri.host?.lowercase().orEmpty())) return false
        return Regex("^/(playlist$|@[^/]+/?(videos|shorts|streams)?/?$|channel/|c/|user/)").containsMatchIn(uri.rawPath.orEmpty())
    }

    /** The whole list's own page for [url], if [url] is a video in a list; else null. */
    fun listOf(url: String): String? {
        val uri = runCatching { java.net.URI(url.trim()) }.getOrNull() ?: return null
        val host = uri.host?.lowercase() ?: return null
        if (!Regex("^(www\\.|m\\.|music\\.)?youtube\\.com$").matches(host) || uri.rawPath != "/watch") return null
        val query = uri.rawQuery.orEmpty().split('&').associate { it.substringBefore('=') to it.substringAfter('=', "") }
        val list = query["list"]?.takeIf { Regex("^[\\w-]{2,64}$").matches(it) } ?: return null
        if (query["v"].isNullOrEmpty()) return null
        return "https://www.youtube.com/playlist?list=$list"
    }
}
