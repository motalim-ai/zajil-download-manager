package io.github.motalim_ai.zajil.browser

import java.net.URI
import java.net.URLEncoder
import org.json.JSONObject

/** What a media request is: a whole file, or a playlist (A6, layer 2). */
enum class MediaKind { File, Hls, Dash }

/**
 * A video the in-app browser found on the page. Everything in it came from
 * the page or its server: untrusted, shown and checked again before any
 * download (the add sheet asks the server, yt-dlp reads the page).
 */
sealed interface Found {
    /** For the list: one entry per address. */
    val key: String

    /** Layer 1: the page is a video site's video page; yt-dlp reads it. */
    data class SitePage(val url: String, val title: String) : Found {
        override val key get() = "page:$url"
    }

    /** Layer 2: a media file or playlist the page asked for. */
    data class Media(val url: String, val kind: MediaKind, val duration: Int = 0) : Found {
        override val key get() = "media:$url"
    }

    /** Layer 3: a `<video>` of the page with its own http(s) address, or a protected one. */
    data class Element(val src: String?, val width: Int, val height: Int, val duration: Int, val protected: Boolean) : Found {
        override val key get() = "element:${src ?: "$width×$height"}"
    }
}

/** A video element as the page's script described it. */
data class PageVideo(val src: String, val width: Int, val height: Int, val duration: Int, val protected: Boolean)

object Finds {
    private val files = setOf("mp4", "m4v", "webm", "mov", "m4a", "mp3", "ogg", "oga", "opus", "flac", "wav", "3gp")

    /**
     * What a request the page made is, by its address's extension: the
     * browser's `shouldInterceptRequest` sees no answer, so no type (the
     * extension's `kindOf` falls back to the same). Only http(s).
     */
    fun kindOf(url: String): MediaKind? {
        val uri = runCatching { URI(url) }.getOrNull() ?: return null
        if (uri.scheme?.lowercase() !in setOf("http", "https") || uri.host.isNullOrEmpty()) return null
        val path = uri.rawPath.orEmpty().lowercase()
        val ext = path.substringAfterLast('/').substringAfterLast('.', "")
        return when {
            ext == "m3u8" -> MediaKind.Hls
            ext == "mpd" -> MediaKind.Dash
            ext in files -> MediaKind.File
            else -> null
        }
    }

    /**
     * Whether a playlist holds one rendition only (its sound, or one size
     * of its picture), as a player loads after the main one: X's
     * `/pl/mp4a/128000/…` and `/pl/avc1/1280x720/…`. From the extension's
     * `isRendition` (apps/extension/src/streams.js).
     */
    fun isRendition(url: String): Boolean {
        val path = runCatching { URI(url).rawPath }.getOrNull()?.lowercase() ?: return false
        return path.split('/').any {
            it.matches(Regex("^(mp4a|avc1|hvc1|hev1|av01|vp09|audio|video)$")) || it.matches(Regex("^\\d{2,5}x\\d{2,5}$"))
        }
    }

    /** The most a page's report may be, and the most videos read from it. */
    const val MAX_REPORT = 64 * 1024
    const val MAX_VIDEOS = 20
    private const val MAX_SRC = 4096

    /**
     * The page script's report ([finder.js]), if it is one: JSON with the
     * load's [token], a title and at most [MAX_VIDEOS] videos. Anything
     * else (another script's message, a forged one) is null. Every field is
     * bounded and checked; an address must be http(s) (a `blob:` one is
     * kept as no address: the playlist behind it is layer 2's).
     */
    fun parseReport(message: String?, token: String): Pair<String, List<PageVideo>>? {
        if (message == null || message.length > MAX_REPORT) return null
        val o = runCatching { JSONObject(message) }.getOrNull() ?: return null
        if (o.optString("t") != token) return null
        val title = o.optString("title").take(200).filterNot { it.isISOControl() }
        val array = o.optJSONArray("videos") ?: return title to emptyList()
        val videos = (0 until minOf(array.length(), MAX_VIDEOS)).mapNotNull { i ->
            val v = array.optJSONObject(i) ?: return@mapNotNull null
            val src = v.optString("src").take(MAX_SRC).let { if (isHttp(it)) it else "" }
            PageVideo(
                src = src,
                width = v.optInt("w").coerceIn(0, 100_000),
                height = v.optInt("h").coerceIn(0, 100_000),
                duration = v.optInt("d").coerceIn(0, 10_000_000),
                protected = v.optBoolean("p"),
            )
        }
        return title to videos
    }

    private fun isHttp(src: String): Boolean {
        val uri = runCatching { URI(src) }.getOrNull() ?: return false
        return uri.scheme?.lowercase() in setOf("http", "https") && !uri.host.isNullOrEmpty()
    }

    /**
     * A host's site, for cookies: its last two labels, or three under a
     * two-letter country's second level («bbc.co.uk»). Cookies go with a
     * found video only when it is the page's own site (never a cross-site frame's).
     */
    fun site(host: String): String {
        // An address (127.0.0.1, [::1]) is its own site.
        if (':' in host || host.matches(Regex("^[\\d.]+$"))) return host.lowercase()
        val labels = host.lowercase().trimEnd('.').split('.').filter { it.isNotEmpty() }
        if (labels.size <= 2) return labels.joinToString(".")
        val last = labels.last()
        val second = labels[labels.size - 2]
        val keep = if (last.length == 2 && second.length <= 3 && second in SECOND_LEVELS) 3 else 2
        return labels.takeLast(keep).joinToString(".")
    }

    private val SECOND_LEVELS = setOf("co", "com", "org", "net", "gov", "edu", "ac", "or", "ne", "go", "gob", "nic")

    fun sameSite(a: String, b: String): Boolean {
        val ha = runCatching { URI(a).host }.getOrNull() ?: return false
        val hb = runCatching { URI(b).host }.getOrNull() ?: return false
        return site(ha) == site(hb)
    }

    /**
     * What the address field opens: an http(s) address as it is, a bare
     * address («example.com/x») over https, anything else a search
     * (DuckDuckGo, which keeps no profile of its users).
     */
    fun addressOf(input: String): String? {
        val text = input.trim()
        if (text.isEmpty()) return null
        val uri = runCatching { URI(text) }.getOrNull()
        if (uri != null && uri.scheme?.lowercase() in setOf("http", "https") && !uri.host.isNullOrEmpty()) return text
        val looksLikeAddress = text.none(Char::isWhitespace) && Regex("^[\\w-]+(\\.[\\w-]+)+(:\\d+)?(/.*)?$").matches(text)
        if (looksLikeAddress) return "https://$text"
        return "https://duckduckgo.com/?q=" + URLEncoder.encode(text, "UTF-8")
    }
}
