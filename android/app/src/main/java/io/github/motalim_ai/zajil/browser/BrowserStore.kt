package io.github.motalim_ai.zajil.browser

import android.content.Context
import android.webkit.CookieManager
import android.webkit.WebStorage
import io.github.motalim_ai.zajil.R
import java.io.File
import java.net.URI
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject

/** A site on the browser's home: its own icon once Zajil has it from the site, else a letter tile; never a logo in the APK. */
data class Shortcut(val name: String, val url: String)

/** A page visited lately (not in private mode). */
data class Visit(val title: String, val url: String)

/** A page in the history (A6b), with when it was opened (milliseconds since 1970). */
data class HistoryEntry(val title: String, val url: String, val at: Long)

/** A page the user kept (A6b). */
data class Bookmark(val title: String, val url: String)

/**
 * The in-app browser's own lists, in the app's private preferences: the
 * home's sites, the pages visited lately, and the sites that keep cookies
 * in the browser (for «Manage sign-ins»), and the sites' icons
 * ([SiteIcons]). Private mode adds nothing here.
 */
class BrowserStore(private val context: Context) {
    private val prefs = context.getSharedPreferences("browser", Context.MODE_PRIVATE)

    /** Each site's own icon, kept until its shortcut, its visits or its data go. */
    val icons = SiteIcons(context, prefs)

    private val _foundFolded = MutableStateFlow(prefs.getBoolean(FOUND_FOLDED, false))

    /** The page's «Download» button folded to a small circle with its count: the user's choice, kept. */
    val foundFolded: StateFlow<Boolean> = _foundFolded.asStateFlow()

    fun setFoundFolded(folded: Boolean) {
        prefs.edit().putBoolean(FOUND_FOLDED, folded).apply()
        _foundFolded.value = folded
    }

    private val _shortcuts = MutableStateFlow(readShortcuts())
    val shortcuts: StateFlow<List<Shortcut>> = _shortcuts.asStateFlow()

    private val _recent = MutableStateFlow(readVisits())
    val recent: StateFlow<List<Visit>> = _recent.asStateFlow()

    private val _sites = MutableStateFlow(prefs.getStringSet(SITES, emptySet()).orEmpty().sorted())
    val sites: StateFlow<List<String>> = _sites.asStateFlow()

    fun addShortcut(shortcut: Shortcut) = saveShortcuts(_shortcuts.value.filterNot { it.url == shortcut.url } + shortcut)

    fun removeShortcut(shortcut: Shortcut) {
        saveShortcuts(_shortcuts.value - shortcut)
        SiteIcons.siteOf(shortcut.url)?.let { site -> if (site !in sitesOf(_shortcuts.value)) icons.remove(site) }
    }

    private val historyFile = File(context.filesDir, "browser-history.json")
    private val io = CoroutineScope(SupervisorJob() + Dispatchers.IO.limitedParallelism(1))

    private val _history = MutableStateFlow<List<HistoryEntry>>(emptyList())

    /** The pages opened, newest first, at most [MAX_HISTORY]; never private mode's. Read from the app's files. */
    val history: StateFlow<List<HistoryEntry>> = _history.asStateFlow()

    private val _bookmarks = MutableStateFlow(readBookmarks())
    val bookmarks: StateFlow<List<Bookmark>> = _bookmarks.asStateFlow()

    init {
        io.launch {
            val read = runCatching {
                JSONArray(historyFile.readText()).let { a ->
                    (0 until minOf(a.length(), MAX_HISTORY)).map { i ->
                        a.getJSONObject(i).let { HistoryEntry(it.getString("title"), it.getString("url"), it.getLong("at")) }
                    }
                }
            }.getOrDefault(emptyList())
            _history.update { (it + read).take(MAX_HISTORY) }
        }
    }

    fun visited(visit: Visit) {
        val list = Finds.withVisit(_recent.value, visit, MAX_RECENT)
        prefs.edit().putString(RECENT, JSONArray(list.map { JSONObject().put("title", it.title).put("url", it.url) }).toString()).apply()
        _recent.value = list
        // The history: every page, the same one again only after another.
        _history.update { old ->
            val entry = HistoryEntry(visit.title, visit.url, System.currentTimeMillis())
            (listOf(entry) + if (old.firstOrNull()?.url == visit.url) old.drop(1) else old).take(MAX_HISTORY)
        }
        saveHistory()
    }

    /** Removes one page from the history. */
    fun forgetVisit(entry: HistoryEntry) {
        _history.update { it - entry }
        saveHistory()
    }

    /** The history and the recent list go, and the icons of the sites in them (not a shortcut's). */
    fun clearRecent() {
        val visited = (_recent.value.map { it.url } + _history.value.map { it.url }).map { Shortcut("", it) }
        (sitesOf(visited) - sitesOf(_shortcuts.value) - sitesOf(_bookmarks.value.map { Shortcut("", it.url) })).forEach(icons::remove)
        prefs.edit().remove(RECENT).apply()
        _recent.value = emptyList()
        _history.value = emptyList()
        io.launch { historyFile.delete() }
    }

    private fun saveHistory() {
        val list = _history.value
        io.launch {
            val temp = File(historyFile.path + ".tmp")
            runCatching {
                temp.writeText(JSONArray(list.map { JSONObject().put("title", it.title).put("url", it.url).put("at", it.at) }).toString())
                if (!temp.renameTo(historyFile)) temp.delete()
            }.onFailure { temp.delete() }
        }
    }

    fun isBookmarked(url: String) = _bookmarks.value.any { it.url == url }

    /** Keeps [url] as a bookmark, or lets it go if it is one. */
    fun toggleBookmark(title: String, url: String) {
        val list = (if (isBookmarked(url)) _bookmarks.value.filterNot { it.url == url } else listOf(Bookmark(title.ifBlank { url }.take(200), url)) + _bookmarks.value).take(MAX_BOOKMARKS)
        prefs.edit().putString(BOOKMARKS, JSONArray(list.map { JSONObject().put("title", it.title).put("url", it.url) }).toString()).apply()
        _bookmarks.value = list
    }

    private fun readBookmarks(): List<Bookmark> = runCatching {
        JSONArray(prefs.getString(BOOKMARKS, "[]")).let { a ->
            (0 until minOf(a.length(), MAX_BOOKMARKS)).map { a.getJSONObject(it).let { o -> Bookmark(o.getString("title"), o.getString("url")) } }
        }
    }.getOrDefault(emptyList())

    private fun sitesOf(list: List<Shortcut>) = list.mapNotNull { SiteIcons.siteOf(it.url) }.toSet()

    /** [url]'s site keeps cookies in the browser now. */
    fun keepsCookies(url: String) {
        val host = runCatching { URI(url).host }.getOrNull() ?: return
        val site = Finds.site(host)
        if (site in _sites.value) return
        val sites = (_sites.value + site).sorted()
        prefs.edit().putStringSet(SITES, sites.toSet()).apply()
        _sites.value = sites
    }

    /**
     * Signs out of [site]: its cookies expire and its pages' storage goes.
     * Android lists cookies by name only, so each name is expired for the
     * site and its «www.» and «m.» hosts, at «/», over https and http.
     */
    fun forget(site: String) {
        val cookies = CookieManager.getInstance()
        for (host in listOf(site, "www.$site", "m.$site")) {
            for (scheme in listOf("https", "http")) {
                val url = "$scheme://$host/"
                cookies.getCookie(url)?.split(';')?.map { it.substringBefore('=').trim() }?.filter { it.isNotEmpty() }?.forEach { name ->
                    cookies.setCookie(url, "$name=; Max-Age=0; Path=/")
                    cookies.setCookie(url, "$name=; Max-Age=0; Path=/; Domain=.$site")
                }
                WebStorage.getInstance().deleteOrigin("$scheme://$host")
            }
        }
        cookies.flush()
        icons.remove(site)
        val sites = _sites.value - site
        prefs.edit().putStringSet(SITES, sites.toSet()).apply()
        _sites.value = sites
    }

    /** Signs out of every site: all the browser's cookies and storage go. */
    fun forgetAll() {
        CookieManager.getInstance().removeAllCookies(null)
        CookieManager.getInstance().flush()
        WebStorage.getInstance().deleteAllData()
        icons.removeAll()
        prefs.edit().remove(SITES).apply()
        _sites.value = emptyList()
    }

    private fun saveShortcuts(list: List<Shortcut>) {
        prefs.edit()
            .putString(SHORTCUTS, JSONArray(list.map { JSONObject().put("name", it.name).put("url", it.url) }).toString())
            .putInt(PRESETS, ShortcutPresets.VERSION)
            .apply()
        _shortcuts.value = list
    }

    private fun readShortcuts(): List<Shortcut> {
        // Never changed: the app's sites, named in the app's language.
        val text = prefs.getString(SHORTCUTS, null) ?: return defaults()
        val stored = runCatching {
            JSONArray(text).let { a -> (0 until a.length()).map { a.getJSONObject(it).let { o -> Shortcut(o.getString("name"), o.getString("url")) } } }
        }.getOrNull() ?: return defaults()
        // Changed by the user: it follows the presets once when they change.
        val from = prefs.getInt(PRESETS, 1)
        if (from >= ShortcutPresets.VERSION) return stored
        val list = ShortcutPresets.migrate(stored, from, presets())
        prefs.edit()
            .putString(SHORTCUTS, JSONArray(list.map { JSONObject().put("name", it.name).put("url", it.url) }).toString())
            .putInt(PRESETS, ShortcutPresets.VERSION)
            .apply()
        val left = list.mapNotNull { SiteIcons.siteOf(it.url) }.toSet()
        ShortcutPresets.droppedSince(from).mapNotNull(SiteIcons::siteOf).filter { it !in left }.forEach(icons::remove)
        return list
    }

    private fun presets() = listOf(
        Preset(context.getString(R.string.site_youtube), "https://m.youtube.com/", since = 1),
        Preset(context.getString(R.string.site_motalim), ShortcutPresets.MOTALIM_CHANNEL, since = 3),
        Preset(context.getString(R.string.site_facebook), "https://m.facebook.com/", since = 1),
        Preset(context.getString(R.string.site_instagram), "https://www.instagram.com/", since = 1),
        Preset("X", "https://x.com/", since = 1),
        Preset(context.getString(R.string.site_tiktok), "https://www.tiktok.com/", since = 1),
        Preset(context.getString(R.string.site_snapchat), "https://www.snapchat.com/spotlight", since = 2),
    )

    private fun defaults() = presets().map { Shortcut(it.name, it.url) }

    private fun readVisits(): List<Visit> = runCatching {
        JSONArray(prefs.getString(RECENT, "[]")).let { a ->
            (0 until a.length()).map { a.getJSONObject(it).let { o -> Visit(o.getString("title"), o.getString("url")) } }
        }
    }.getOrDefault(emptyList())

    companion object {
        private const val SHORTCUTS = "shortcuts"
        private const val FOUND_FOLDED = "found_folded"
        private const val PRESETS = "presets_version"
        private const val RECENT = "recent"
        private const val BOOKMARKS = "bookmarks"
        private const val MAX_HISTORY = 500
        private const val MAX_BOOKMARKS = 200
        private const val SITES = "sites"
        private const val MAX_RECENT = 8
    }
}
