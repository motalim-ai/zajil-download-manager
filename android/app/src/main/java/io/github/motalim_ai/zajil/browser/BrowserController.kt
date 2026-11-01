package io.github.motalim_ai.zajil.browser

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Bitmap
import android.net.http.SslError
import android.os.Handler
import android.os.Looper
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.GeolocationPermissions
import android.webkit.PermissionRequest
import android.webkit.RenderProcessGoneDetail
import android.webkit.ServiceWorkerClient
import android.webkit.ServiceWorkerController
import android.webkit.SslErrorHandler
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.webkit.ProfileStore
import androidx.webkit.ServiceWorkerClientCompat
import androidx.webkit.ServiceWorkerControllerCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import io.github.motalim_ai.zajil.BuildConfig
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.downloads.BrowserLink
import io.github.motalim_ai.zajil.downloads.CookieSource
import io.github.motalim_ai.zajil.protection
import io.github.motalim_ai.zajil.video.VideoPages
import java.security.SecureRandom
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONObject

/** The page on screen, or null for the browser's home. */
data class PageState(
    val url: String,
    val title: String = "",
    val loading: Boolean = true,
    val progress: Int = 0,
    val canGoBack: Boolean = false,
    val canGoForward: Boolean = false,
)

/** A tab, as the switcher lists it. */
data class TabInfo(val id: Long, val title: String, val url: String)

/** A harmful page the browser stopped before it loaded: [url], and why. */
data class Warning(val url: String, val host: String, val guard: Guard)

/**
 * Zajil's in-app browser (A6; tabs from 0.3.0): a WebView for each tab,
 * with the settings the owner set as hard rules. It finds videos in three
 * layers (a video site's page; media files and playlists the page asks
 * for; the page's `<video>` elements, reported by [finder.js]), and starts
 * nothing itself: a download starts only from the user's tap in Zajil. The
 * screen follows the current tab; none is the browser's home.
 */
@SuppressLint("SetJavaScriptEnabled")
class BrowserController(private val context: Context) {
    val store = BrowserStore(context)
    private val protection = context.protection

    private val _blocked = MutableStateFlow(0)

    /** Ads, trackers and harmful requests not made on the current tab's page (in memory only). */
    val blocked: StateFlow<Int> = _blocked.asStateFlow()

    private val _warning = MutableStateFlow<Warning?>(null)

    /** The current tab's harmful page, stopped before it loaded. */
    val warning: StateFlow<Warning?> = _warning.asStateFlow()

    private val _page = MutableStateFlow<PageState?>(null)
    val page: StateFlow<PageState?> = _page.asStateFlow()

    private val _found = MutableStateFlow<List<Found>>(emptyList())
    val found: StateFlow<List<Found>> = _found.asStateFlow()

    private val _private = MutableStateFlow(false)
    val privateMode: StateFlow<Boolean> = _private.asStateFlow()

    private val _tabs = MutableStateFlow<List<TabInfo>>(emptyList())
    val tabs: StateFlow<List<TabInfo>> = _tabs.asStateFlow()

    /** A link the page wanted to download: offered in Zajil's sheet, never started. */
    private val _requests = MutableSharedFlow<Pair<String, BrowserLink?>>(extraBufferCapacity = 4)
    val requests: SharedFlow<Pair<String, BrowserLink?>> = _requests.asSharedFlow()

    private val main = Handler(Looper.getMainLooper())

    /** The page script's secret for its reports (never in a page), and its channel's name. */
    private val token = random()
    private val bridge = "zajil_" + random().take(12)

    /** A tab: its WebView, its page, and what was found on it, by layer (main thread). */
    private inner class Tab(val id: Long, val view: WebView) {
        var page: PageState? = null
        var found: List<Found> = emptyList()
        var sitePage: Found.SitePage? = null
        val media = LinkedHashMap<String, Found.Media>()
        val elements = HashMap<String, List<PageVideo>>()

        /** The page's site, for the requests' thread. */
        @Volatile var site: String? = null

        /** A page's request was seen since the last page started: its count started there. */
        @Volatile var requested = false
        val blocked = AtomicInteger()
        var warning: Warning? = null

        /** Harmful hosts the user chose to open anyway, in this tab only. */
        val bypass: MutableSet<String> = ConcurrentHashMap.newKeySet()
    }

    private val tabList = mutableListOf<Tab>()
    private var current: Tab? = null
    private var nextId = 1L

    /** The current tab's WebView; null on the home. */
    val webView: WebView? get() = current?.view

    val privateSupported: Boolean
        get() = WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)

    val detectsElements: Boolean
        get() = WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT) &&
            WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)

    /** Opens what the user typed or tapped: in the current tab, or a new one from the home. */
    fun open(input: String) {
        val url = Finds.addressOf(input) ?: return
        val tab = current ?: newTab()
        setPage(tab, PageState(url))
        tab.view.loadUrl(url)
    }

    /** A new tab: the home, where the next address opens in a new tab. */
    fun newTabHome() {
        current?.view?.onPause()
        current = null
        _page.value = null
        _found.value = emptyList()
        _blocked.value = 0
        _warning.value = null
    }

    /** At most [MAX_TABS]: the oldest goes first. */
    private fun newTab(): Tab {
        if (tabList.size >= MAX_TABS) closeTab(tabList.first().id)
        val tab = Tab(nextId++, create())
        synchronized(tabList) { tabList += tab }
        current = tab
        listTabs()
        return tab
    }

    /** Shows tab [id]. */
    fun switchTo(id: Long) {
        val tab = tabList.firstOrNull { it.id == id } ?: return
        current?.takeIf { it != tab }?.view?.onPause()
        current = tab
        tab.view.onResume()
        _page.value = tab.page
        _found.value = tab.found
        _blocked.value = tab.blocked.get()
        _warning.value = tab.warning
    }

    /** The harmful page anyway, in this tab only: the user's choice («on my own responsibility»). */
    fun proceedAnyway() {
        val tab = current ?: return
        val warning = tab.warning ?: return
        tab.bypass += warning.host
        tab.warning = null
        _warning.value = null
        tab.view.loadUrl(warning.url)
    }

    /** Away from the harmful page: back, or the tab closes. */
    fun leaveWarning() {
        val tab = current ?: return
        tab.warning = null
        _warning.value = null
        back()
    }

    /** The current page's site, for the protection sheet. */
    val site: String? get() = current?.site

    /** Closes tab [id]; the current one gives way to the last other tab, or the home. */
    fun closeTab(id: Long) {
        val tab = tabList.firstOrNull { it.id == id } ?: return
        synchronized(tabList) { tabList -= tab }
        tab.view.stopLoading()
        tab.view.destroy()
        listTabs()
        if (tab == current) {
            current = null
            tabList.lastOrNull()?.let { switchTo(it.id) } ?: run {
                _page.value = null
                _found.value = emptyList()
                _blocked.value = 0
                _warning.value = null
            }
        }
    }

    fun reload() = webView?.reload()

    /** Back in the tab's history, or the tab closes. */
    fun back() {
        val tab = current ?: return
        if (tab.view.canGoBack()) tab.view.goBack() else closeTab(tab.id)
    }

    /** Forward in the tab's history, if the user went back. */
    fun forward() {
        current?.view?.takeIf { it.canGoForward() }?.goForward()
    }

    /** Closes every tab: the home. */
    fun close() {
        tabList.toList().forEach { closeTab(it.id) }
    }

    private fun listTabs() {
        _tabs.value = tabList.map { TabInfo(it.id, it.page?.title.orEmpty(), it.page?.url.orEmpty()) }
    }

    /** [tab]'s page changed: the screen shows it if it is the current tab. */
    private fun setPage(tab: Tab, page: PageState?) {
        val before = tab.page
        tab.page = page
        tab.site = page?.url?.let { runCatching { java.net.URI(it).host }.getOrNull() }?.lowercase()?.let(Finds::site)
        if (tab == current) _page.value = page
        if (before?.title != page?.title || before?.url != page?.url) listTabs()
    }

    /**
     * Private mode on or off: its pages run in a WebView profile of their
     * own (cookies, storage, cache), deleted when it ends; nothing is added
     * to the home's lists. False if this phone's WebView has no profiles.
     */
    fun setPrivate(on: Boolean): Boolean {
        if (on == _private.value) return true
        if (on && !privateSupported) return false
        close()
        // Off: its data goes. On: so does any left from a session the app
        // never ended (killed, or a profile still held when it closed).
        deletePrivateProfile()
        _private.value = on
        return true
    }

    fun onPause() = webView?.onPause()

    fun onResume() = webView?.onResume()

    fun destroy() {
        close()
        if (_private.value) deletePrivateProfile()
    }

    /**
     * What goes with a download of [url] found on the page on screen: the
     * page (sent as the browsers send a Referer, without its fragment or
     * user name), the browser's User-Agent, and its cookies for [url] only
     * when [url] is the page's own site (never a cross-site frame's), as they are now; private mode's never outlive it.
     */
    fun linkFor(url: String): BrowserLink? = current?.let { linkFor(url, it) }

    /** [linkFor] on [tab]'s page: the tab the link came from, not the one on screen. */
    private fun linkFor(url: String, tab: Tab): BrowserLink? {
        val pageUrl = tab.page?.url ?: return null
        val same = Finds.sameSite(url, pageUrl)
        return BrowserLink(
            url = url,
            page = Finds.referrer(pageUrl),
            source = when {
                !same -> CookieSource.None
                _private.value -> CookieSource.Private
                else -> CookieSource.Browser
            },
            userAgent = tab.view.settings.userAgentString,
            cookies = if (same) cookieManager()?.getCookie(url) else null,
        )
    }

    /**
     * A page's service worker asks for things without the page's
     * WebViewClient: the same guards here. A harmful host is never asked;
     * ads and trackers of another site than the worker's page (known by its
     * Referer) are not, where they are not let through. And the worker gets
     * no file or content access, as the pages; private mode's, no cache.
     */
    private fun guardWorkers(controller: ServiceWorkerControllerCompat, private: Boolean) {
        controller.serviceWorkerWebSettings.apply {
            if (WebViewFeature.isFeatureSupported(WebViewFeature.SERVICE_WORKER_CONTENT_ACCESS)) allowContentAccess = false
            if (WebViewFeature.isFeatureSupported(WebViewFeature.SERVICE_WORKER_FILE_ACCESS)) allowFileAccess = false
            if (private && WebViewFeature.isFeatureSupported(WebViewFeature.SERVICE_WORKER_CACHE_MODE)) cacheMode = WebSettings.LOAD_NO_CACHE
        }
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.SERVICE_WORKER_SHOULD_INTERCEPT_REQUEST)) return
        controller.setServiceWorkerClient(object : ServiceWorkerClientCompat() {
            override fun shouldInterceptRequest(request: WebResourceRequest): WebResourceResponse? = workerRequest(request)
        })
    }

    /** [guardWorkers] for private mode's profile, whose controller is the platform's own. */
    private fun guardWorkers(controller: ServiceWorkerController) {
        controller.serviceWorkerWebSettings.apply {
            allowContentAccess = false
            allowFileAccess = false
            cacheMode = WebSettings.LOAD_NO_CACHE
        }
        controller.setServiceWorkerClient(object : ServiceWorkerClient() {
            override fun shouldInterceptRequest(request: WebResourceRequest): WebResourceResponse? = workerRequest(request)
        })
    }

    /** A service worker's request: an empty answer if a guard stops it, else null (asked as it is). */
    private fun workerRequest(request: WebResourceRequest): WebResourceResponse? {
        val host = request.url.host?.lowercase() ?: return null
        if (protection.harm(host, Finds.site(host)) != null) {
            protection.blocked(ListKind.Malware)
            return EMPTY()
        }
        val site = request.requestHeaders["Referer"]
            ?.let { runCatching { java.net.URI(it).host }.getOrNull() }
            ?.lowercase()?.let(Finds::site) ?: return null
        if (Finds.site(host) != site && protection.settings.value.active(Guard.Ads, site)) {
            protection.adKind(host)?.let { kind ->
                protection.blocked(kind)
                return EMPTY()
            }
        }
        return null
    }

    private fun cookieManager(): CookieManager? = if (_private.value) {
        runCatching { ProfileStore.getInstance().getProfile(PRIVATE)?.cookieManager }.getOrNull()
    } else {
        CookieManager.getInstance()
    }

    private fun deletePrivateProfile() {
        if (!privateSupported) return
        val store = ProfileStore.getInstance()
        val profile = store.getProfile(PRIVATE) ?: return
        // A WebView destroyed a moment ago may still hold the profile, and a
        // profile in use cannot be deleted: its cookies and storage go now
        // regardless, and the profile itself once it is free.
        profile.cookieManager.removeAllCookies(null)
        profile.webStorage.deleteAllData()
        if (runCatching { store.deleteProfile(PRIVATE) }.isFailure) {
            main.post { runCatching { store.deleteProfile(PRIVATE) } }
        }
    }

    private fun create(): WebView {
        // DevTools over adb, in the debug build only: how a site's blank page is diagnosed.
        if (BuildConfig.DEBUG) WebView.setWebContentsDebuggingEnabled(true)
        val view = WebView(context)
        if (_private.value) {
            val profile = ProfileStore.getInstance().getOrCreateProfile(PRIVATE)
            WebViewCompat.setProfile(view, PRIVATE)
            runCatching { guardWorkers(profile.serviceWorkerController) }
        } else if (WebViewFeature.isFeatureSupported(WebViewFeature.SERVICE_WORKER_BASIC_USAGE)) {
            guardWorkers(ServiceWorkerControllerCompat.getInstance(), private = false)
        }
        view.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            // The owner's rules: no file or content access, never mixed
            // content, Safe Browsing on.
            allowFileAccess = false
            allowContentAccess = false
            @Suppress("DEPRECATION")
            allowFileAccessFromFileURLs = false
            @Suppress("DEPRECATION")
            allowUniversalAccessFromFileURLs = false
            mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
            safeBrowsingEnabled = true
            setGeolocationEnabled(false)
            // No pop-up windows; a page's video waits for a tap to play.
            setSupportMultipleWindows(false)
            javaScriptCanOpenWindowsAutomatically = false
            mediaPlaybackRequiresUserGesture = true
            builtInZoomControls = true
            displayZoomControls = false
            if (_private.value) cacheMode = WebSettings.LOAD_NO_CACHE
        }
        // The page's height is the screen's, never «wrap content»: Compose's
        // AndroidView gives a view without layout params WRAP_CONTENT, and
        // WebView then makes 100vh zero, so Facebook's login (100vh) was a
        // blank white page (owner's report, 2026-11-01).
        view.layoutParams = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
        // Third-party cookies stay off (WebView's default), in both modes.
        cookieManager()?.setAcceptThirdPartyCookies(view, false)
        view.webViewClient = Client()
        view.webChromeClient = Chrome()
        // A page's download: offered in Zajil's sheet, never started by the page.
        // Only the tab on screen: a tab in the background cannot open the
        // sheet over another, nor take the page on screen with it.
        view.setDownloadListener { url, _, _, _, _ ->
            val tab = tabOf(view)
            if (tab != null && tab == current && (url.startsWith("https://") || url.startsWith("http://"))) {
                _requests.tryEmit(url to linkFor(url, tab))
            }
        }
        if (detectsElements) {
            val script = context.assets.open("browser/finder.js").bufferedReader().use { it.readText() }
                .replace("__ZAJIL_BRIDGE__", bridge)
                .replace("__ZAJIL_TOKEN__", JSONObject.quote(token))
                .replace("__ZAJIL_LABEL__", JSONObject.quote(context.getString(R.string.found_by_zajil)))
            WebViewCompat.addDocumentStartJavaScript(view, script, setOf("*"))
            WebViewCompat.addWebMessageListener(view, bridge, setOf("*")) { from, message, origin, isMainFrame, _ ->
                // Untrusted page data: only a report with this load's token, bounded and checked.
                val report = Finds.parseReport(message.data, token) ?: return@addWebMessageListener
                val tab = tabOf(from) ?: return@addWebMessageListener
                tab.elements["$isMainFrame|$origin"] = report.second
                if (isMainFrame && report.first.isNotBlank()) setPage(tab, tab.page?.copy(title = report.first))
                publish(tab)
            }
        }
        return view
    }

    private fun tabOf(view: WebView): Tab? = tabList.firstOrNull { it.view == view }

    /** One more request not made on [tab]'s page. */
    private fun count(tab: Tab) {
        val n = tab.blocked.incrementAndGet()
        main.post { if (tab == current) _blocked.value = n }
    }

    private fun warn(tab: Tab, warning: Warning) {
        tab.warning = warning
        if (tab == current) _warning.value = warning
    }

    /** The page's address changed: layer 1 again, and what it asks for from now on. */
    private fun moved(tab: Tab, url: String) {
        tab.sitePage = if (VideoPages.isVideoPage(url)) Found.SitePage(url, tab.page?.title.orEmpty()) else null
        tab.media.clear()
        publish(tab)
    }

    /**
     * The list for the «Download» button. On a video site's page, the page
     * alone: yt-dlp reads it whole (its media requests are pieces). Else
     * the media files and main playlists the page asked for (a rendition
     * only when there is no main one), then its videos with an address of
     * their own, then the protected ones, which are not downloaded.
     */
    private fun publish(tab: Tab) {
        tab.found = found(tab)
        if (tab == current) _found.value = tab.found
    }

    private fun found(tab: Tab): List<Found> {
        tab.sitePage?.let { return listOf(it) }
        // A feed (TikTok's «For you», Instagram's, Facebook's): its videos
        // by their own posts' pages, which yt-dlp reads; the files a feed
        // plays are pieces. Only the page's own site's.
        val pageUrl = tab.page?.url
        val posts = tab.elements.values.flatten().map { it.page }
            .filter { it.isNotEmpty() && pageUrl != null && Finds.sameSite(it, pageUrl) }
            .distinct()
        if (posts.isNotEmpty()) return posts.take(Finds.MAX_VIDEOS).map { Found.SitePage(it, "") }
        val list = mutableListOf<Found>()
        val playlists = tab.media.values.filter { it.kind != MediaKind.File }
        val mains = playlists.filterNot { Finds.isRendition(it.url) }
        list += (mains.ifEmpty { playlists.take(1) })
        val videos = tab.elements.values.flatten()
        // A file the page's <video> plays: its length from there.
        list += tab.media.values.filter { it.kind == MediaKind.File }.map { file ->
            val length = videos.firstOrNull { it.src.substringBefore('?') == file.url.substringBefore('?') }?.duration ?: 0
            if (length > 0) file.copy(duration = length) else file
        }
        val known = list.map { (it as Found.Media).url.substringBefore('?') }.toSet()
        videos.filter { it.src.isNotEmpty() && !it.protected && it.src.substringBefore('?') !in known }
            .distinctBy { it.src }
            .forEach { list += Found.Element(it.src, it.width, it.height, it.duration, protected = false) }
        videos.filter { it.protected }.take(1).forEach { list += Found.Element(null, it.width, it.height, it.duration, protected = true) }
        return list
    }

    private inner class Client : WebViewClient() {
        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
            // Only http(s): no intent:, file:, content: or another app's links.
            val scheme = request.url.scheme?.lowercase()
            if (scheme != "http" && scheme != "https") return true
            // A harmful page is warned about even if a service worker would answer it
            // (its request would never reach shouldInterceptRequest).
            val host = request.url.host?.lowercase()?.trimEnd('.') ?: return false
            val tab = tabOf(view) ?: return false
            if (request.isForMainFrame && host !in tab.bypass) {
                protection.harm(host, Finds.site(host))?.let { guard ->
                    protection.blocked(ListKind.Malware)
                    warn(tab, Warning(request.url.toString(), host, guard))
                    return true
                }
            }
            return false
        }

        override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) {
            val tab = tabOf(view) ?: return
            tab.elements.clear()
            // A page from the cache (back, forward) has no request: its count starts here.
            if (!tab.requested) {
                tab.blocked.set(0)
                if (tab == current) _blocked.value = 0
            }
            tab.requested = false
            setPage(tab, PageState(url, loading = true, canGoBack = view.canGoBack(), canGoForward = view.canGoForward()))
            moved(tab, url)
        }

        override fun doUpdateVisitedHistory(view: WebView, url: String, isReload: Boolean) {
            val tab = tabOf(view) ?: return
            val before = tab.page?.url
            setPage(tab, tab.page?.copy(url = url, canGoBack = view.canGoBack(), canGoForward = view.canGoForward()) ?: PageState(url))
            if (url != before) moved(tab, url)
        }

        override fun onPageFinished(view: WebView, url: String) {
            val tab = tabOf(view) ?: return
            setPage(tab, tab.page?.copy(loading = false, progress = 100, canGoBack = view.canGoBack(), canGoForward = view.canGoForward()))
            tab.sitePage = tab.sitePage?.copy(title = view.title.orEmpty())
            publish(tab)
            // Private mode: no history, no recent list, no sites (A6b).
            if (_private.value || !url.startsWith("http")) return
            store.visited(Visit(view.title?.takeIf { it.isNotBlank() } ?: url, url))
            if (!CookieManager.getInstance().getCookie(url).isNullOrEmpty()) store.keepsCookies(url)
        }

        /**
         * The protection (0.3.0), then layer 2: a media file or playlist the
         * page asks for, only looked at, never changed or fetched. Called on
         * WebView's threads: the lists answer from memory.
         */
        override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? {
            val host = request.url.host?.lowercase() ?: return null
            val tab = synchronized(tabList) { tabList.firstOrNull { it.view == view } } ?: return null
            if (request.isForMainFrame) {
                // A new page: its site and count are set here, on the requests' thread, as its
                // parts may be asked for before onPageStarted reaches the main thread.
                tab.site = Finds.site(host)
                tab.requested = true
                tab.blocked.set(0)
                main.post { if (tab == current) _blocked.value = 0 }
            }
            if (host !in tab.bypass) {
                // A harmful site: a page is stopped and warned about; a part of one is not made.
                protection.harm(host, Finds.site(host))?.let { guard ->
                    protection.blocked(ListKind.Malware)
                    if (request.isForMainFrame) {
                        main.post { warn(tab, Warning(request.url.toString(), host, guard)) }
                    } else {
                        count(tab)
                    }
                    return EMPTY()
                }
            }
            if (request.isForMainFrame) return null
            // Ads and trackers of another site than the page's, where they are not let through.
            val site = tab.site
            if (site != null && Finds.site(host) != site && protection.settings.value.active(Guard.Ads, site)) {
                protection.adKind(host)?.let { kind ->
                    count(tab)
                    protection.blocked(kind)
                    return EMPTY()
                }
            }
            val url = request.url.toString()
            val kind = Finds.kindOf(url) ?: return null
            main.post {
                val tab = tabOf(view) ?: return@post
                val key = url.substringBefore('?')
                if (key !in tab.media && tab.media.size < MAX_MEDIA) {
                    tab.media[key] = Found.Media(url, kind)
                    publish(tab)
                }
            }
            return null
        }

        /** A certificate Android does not trust: never shown, as everywhere in Zajil. */
        override fun onReceivedSslError(view: WebView, handler: SslErrorHandler, error: SslError) = handler.cancel()

        override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
            tabOf(view)?.let { closeTab(it.id) } ?: view.destroy()
            return true
        }
    }

    private inner class Chrome : WebChromeClient() {
        /** The page's own icon, kept for its site's tile and row; never in private mode. */
        override fun onReceivedIcon(view: WebView, icon: Bitmap?) {
            val url = view.url ?: return
            if (icon == null || _private.value || !url.startsWith("http")) return
            store.icons.fromPage(url, icon)
        }

        override fun onProgressChanged(view: WebView, newProgress: Int) {
            val tab = tabOf(view) ?: return
            setPage(tab, tab.page?.copy(progress = newProgress, loading = newProgress < 100))
        }

        override fun onReceivedTitle(view: WebView, title: String?) {
            val tab = tabOf(view) ?: return
            setPage(tab, tab.page?.copy(title = title.orEmpty()))
        }

        /** No camera, microphone or other device for pages. */
        override fun onPermissionRequest(request: PermissionRequest) = request.deny()

        override fun onGeolocationPermissionsShowPrompt(origin: String?, callback: GeolocationPermissions.Callback) =
            callback.invoke(origin, false, false)

        /** No file of the phone for pages. */
        override fun onShowFileChooser(
            webView: WebView,
            filePathCallback: ValueCallback<Array<android.net.Uri>>,
            fileChooserParams: FileChooserParams,
        ): Boolean {
            filePathCallback.onReceiveValue(null)
            return true
        }
    }

    private companion object {
        const val PRIVATE = "zajil-private"

        /** The most media addresses kept for a page. */
        const val MAX_MEDIA = 20

        /** The most tabs open: each is a WebView, and a phone's memory is short. */
        const val MAX_TABS = 10

        /** What a request that is not made gets: nothing. */
        fun EMPTY() = WebResourceResponse("text/plain", "utf-8", java.io.ByteArrayInputStream(ByteArray(0)))

        fun random(): String {
            val bytes = ByteArray(16)
            SecureRandom().nextBytes(bytes)
            return bytes.joinToString("") { "%02x".format(it) }
        }
    }
}
