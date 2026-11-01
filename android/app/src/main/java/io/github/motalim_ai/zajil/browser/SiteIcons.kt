package io.github.motalim_ai.zajil.browser

import android.content.Context
import android.content.SharedPreferences
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.webkit.WebSettings
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import java.io.ByteArrayOutputStream
import java.io.File
import java.net.HttpURLConnection
import java.net.URI
import java.net.URL
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * Each site's own icon for the browser's home, by site (owner's request,
 * 2026-11-01: «each site's icon instead of its first letter»), in the
 * app's private files. From the page as it loads (WebView's icon), or,
 * for a shortcut not visited yet, asked once of that site itself
 * ([IconLinks]). Never a logo in the APK, never a favicon service, nothing
 * in private mode. A site with no icon keeps its letter tile.
 */
class SiteIcons(private val context: Context, private val prefs: SharedPreferences) {
    private val dir = File(context.filesDir, "site-icons")
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private val _icons = MutableStateFlow<Map<String, ImageBitmap>>(emptyMap())

    /** The icons kept, by site ([Finds.site]). */
    val icons: StateFlow<Map<String, ImageBitmap>> = _icons.asStateFlow()

    init {
        scope.launch {
            val read = dir.listFiles().orEmpty().filter { it.name.endsWith(".png") }.mapNotNull { file ->
                val bitmap = runCatching { BitmapFactory.decodeFile(file.path) }.getOrNull() ?: return@mapNotNull null
                file.name.removeSuffix(".png") to bitmap.asImageBitmap()
            }.toMap()
            _icons.update { read + it }
        }
    }

    /** The icon the page at [pageUrl] gave WebView: its site's, from now on. */
    fun fromPage(pageUrl: String, icon: Bitmap) {
        val site = siteOf(pageUrl) ?: return
        // WebView may reuse its bitmap: a copy, made now.
        val copy = runCatching { icon.copy(Bitmap.Config.ARGB_8888, false) }.getOrNull() ?: return
        scope.launch { keep(site, copy) }
    }

    /**
     * The shortcuts' sites with no icon yet: each asked once (and never
     * again until its icon is removed), from that site only.
     */
    fun fetchMissing(urls: List<String>) {
        val tried = prefs.getStringSet(TRIED, emptySet()).orEmpty()
        val wanted = urls.mapNotNull { url -> siteOf(url)?.let { it to url } }.distinctBy { it.first }
            .filter { (site, _) -> site !in _icons.value && site !in tried }
        if (wanted.isEmpty()) return
        prefs.edit().putStringSet(TRIED, tried + wanted.map { it.first }).apply()
        for ((site, url) in wanted) scope.launch { fetch(url)?.let { keep(site, it) } }
    }

    /** [site]'s icon goes, and may be asked for again. */
    fun remove(site: String) {
        _icons.update { it - site }
        prefs.edit().putStringSet(TRIED, prefs.getStringSet(TRIED, emptySet()).orEmpty() - site).apply()
        IconLinks.fileName(site)?.let { name -> scope.launch { File(dir, name).delete() } }
    }

    fun removeAll() {
        _icons.value = emptyMap()
        prefs.edit().remove(TRIED).apply()
        scope.launch { dir.listFiles().orEmpty().forEach { it.delete() } }
    }

    private fun keep(site: String, icon: Bitmap) {
        val name = IconLinks.fileName(site) ?: return
        val small = scaled(icon) ?: return
        // A page's 16-pixel favicon never replaces a sharper icon of the site's.
        if ((_icons.value[site]?.width ?: 0) > small.width) return
        val bytes = ByteArrayOutputStream().also { small.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        dir.mkdirs()
        val temp = File(dir, "$name.tmp")
        runCatching {
            temp.writeBytes(bytes)
            if (!temp.renameTo(File(dir, name))) temp.delete()
        }.onFailure { temp.delete() }
        _icons.update { it + (site to small.asImageBitmap()) }
    }

    /**
     * [page]'s own site's icon: the first that decodes at [GOOD] pixels or
     * more, else the largest found (a 16-pixel favicon is blurry on a
     * tile), or null. Never more than [IconLinks.MAX_BYTES] read each.
     */
    private fun fetch(page: String): Bitmap? {
        val html = read(page)?.toString(Charsets.UTF_8)
        var best: Bitmap? = null
        for (url in IconLinks.candidates(page, html)) {
            val icon = read(url)?.let(::decode) ?: continue
            if (best == null || icon.width > best.width) best = icon
            if (icon.width >= GOOD) break
        }
        return best
    }

    /** [url]'s body: https only, redirects only within [url]'s site, at most [IconLinks.MAX_BYTES], no cookies. */
    private fun read(url: String): ByteArray? {
        var at = url
        repeat(4) {
            if (!IconLinks.acceptable(at, url)) return null
            val connection = runCatching { URL(at).openConnection() as HttpURLConnection }.getOrNull() ?: return null
            try {
                connection.instanceFollowRedirects = false
                connection.connectTimeout = 10_000
                connection.readTimeout = 10_000
                connection.useCaches = false
                connection.setRequestProperty("User-Agent", userAgent)
                val code = connection.responseCode
                if (code in 300..399) {
                    val next = connection.getHeaderField("Location") ?: return null
                    at = runCatching { URI(at).resolve(next).toString() }.getOrNull() ?: return null
                    return@repeat
                }
                if (code != 200 || connection.contentLengthLong > IconLinks.MAX_BYTES) return null
                connection.inputStream.use { input ->
                    val out = ByteArrayOutputStream()
                    val buffer = ByteArray(16 * 1024)
                    while (true) {
                        val n = input.read(buffer)
                        if (n < 0) break
                        if (out.size() + n > IconLinks.MAX_BYTES) return null
                        out.write(buffer, 0, n)
                    }
                    return out.toByteArray()
                }
            } catch (_: Exception) {
                return null
            } finally {
                connection.disconnect()
            }
        }
        return null
    }

    private val userAgent by lazy { runCatching { WebSettings.getDefaultUserAgent(context) }.getOrDefault("Mozilla/5.0") }

    companion object {
        private const val TRIED = "icons_tried"

        /** The side icons are kept at: enough for a 56dp tile's 34dp picture. */
        private const val SIDE = 96

        /** An icon sharp enough on a tile: no further one is asked for. */
        private const val GOOD = 64

        fun siteOf(url: String): String? {
            val uri = runCatching { URI(url) }.getOrNull() ?: return null
            if (uri.scheme?.lowercase() !in setOf("http", "https")) return null
            return uri.host?.let(Finds::site)?.takeIf { IconLinks.fileName(it) != null }
        }

        /** An image from a site, decoded safely: its size read first, never more than about [SIDE] kept. */
        private fun decode(bytes: ByteArray): Bitmap? {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
            if (bounds.outWidth !in 1..4096 || bounds.outHeight !in 1..4096) return null
            var sample = 1
            while (bounds.outWidth / (sample * 2) >= SIDE && bounds.outHeight / (sample * 2) >= SIDE) sample *= 2
            val options = BitmapFactory.Options().apply { inSampleSize = sample }
            return runCatching { BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options) }.getOrNull()
        }

        private fun scaled(icon: Bitmap): Bitmap? {
            if (icon.width <= 0 || icon.height <= 0) return null
            val side = maxOf(icon.width, icon.height)
            if (side <= SIDE) return icon
            val k = SIDE.toFloat() / side
            return Bitmap.createScaledBitmap(icon, maxOf(1, (icon.width * k).toInt()), maxOf(1, (icon.height * k).toInt()), true)
        }
    }
}
