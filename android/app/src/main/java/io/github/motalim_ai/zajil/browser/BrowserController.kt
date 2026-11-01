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
import android.webkit.SslErrorHandler
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.webkit.ProfileStore
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import io.github.motalim_ai.zajil.BuildConfig
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.downloads.BrowserLink
import io.github.motalim_ai.zajil.downloads.CookieSource
import io.github.motalim_ai.zajil.video.VideoPages
import java.security.SecureRandom
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
)

/**
 * Zajil's in-app browser (A6): one WebView, held for the window, with the
 * settings the owner set as hard rules. It finds videos in three layers
 * (a video site's page; media files and playlists the page asks for; the
 * page's `<video>` elements, reported by [finder.js]), and starts nothing
 * itself: a download starts only from the user's tap in Zajil.
 */
@SuppressLint("SetJavaScriptEnabled")
class BrowserController(private val context: Context) {
    val store = BrowserStore(context)

    private val _page = MutableStateFlow<PageState?>(null)
    val page: StateFlow<PageState?> = _page.asStateFlow()

    private val _found = MutableStateFlow<List<Found>>(emptyList())
    val found: StateFlow<List<Found>> = _found.asStateFlow()

    private val _private = MutableStateFlow(false)
    val privateMode: StateFlow<Boolean> = _private.asStateFlow()

    /** A link the page wanted to download: offered in Zajil's sheet, never started. */
    private val _requests = MutableSharedFlow<String>(extraBufferCapacity = 4)
    val requests: SharedFlow<String> = _requests.asSharedFlow()

    var webView: WebView? = null
        private set

    private val main = Handler(Looper.getMainLooper())

    /** The page script's secret for its reports (never in a page), and its channel's name. */
    private val token = random()
    private val bridge = "zajil_" + random().take(12)

    /** What was found, by layer, on the page on screen (main thread). */
    private var sitePage: Found.SitePage? = null
    private val media = LinkedHashMap<String, Found.Media>()
    private val elements = HashMap<String, List<PageVideo>>()

    val privateSupported: Boolean
        get() = WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)

    val detectsElements: Boolean
        get() = WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT) &&
            WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)

    /** Opens what the user typed or tapped: an address, or a search. */
    fun open(input: String) {
        val url = Finds.addressOf(input) ?: return
        val view = webView ?: create().also { webView = it }
        _page.value = PageState(url)
        view.loadUrl(url)
    }

    fun reload() = webView?.reload()

    /** Back in the page's history, or to the browser's home. */
    fun back() {
        val view = webView ?: return
        if (view.canGoBack()) view.goBack() else close()
    }

    /** Back to the browser's home; the page goes. */
    fun close() {
        webView?.let {
            it.stopLoading()
            it.destroy()
        }
        webView = null
        _page.value = null
        reset()
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
     * page (sent as the browsers send a Referer), the browser's
     * User-Agent, and its cookies for [url] only when [url] is the page's
     * own site (never a cross-site frame's), as they are
     * now; private mode's never outlive it.
     */
    fun linkFor(url: String): BrowserLink? {
        val pageUrl = _page.value?.url ?: return null
        val cookies = if (Finds.sameSite(url, pageUrl)) cookieManager()?.getCookie(url) else null
        return BrowserLink(
            page = pageUrl,
            source = if (_private.value) CookieSource.Private else CookieSource.Browser,
            userAgent = webView?.settings?.userAgentString,
            cookies = cookies,
        )
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
            ProfileStore.getInstance().getOrCreateProfile(PRIVATE)
            WebViewCompat.setProfile(view, PRIVATE)
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
        view.setDownloadListener { url, _, _, _, _ ->
            if (url.startsWith("https://") || url.startsWith("http://")) _requests.tryEmit(url)
        }
        if (detectsElements) {
            val script = context.assets.open("browser/finder.js").bufferedReader().use { it.readText() }
                .replace("__ZAJIL_BRIDGE__", bridge)
                .replace("__ZAJIL_TOKEN__", JSONObject.quote(token))
                .replace("__ZAJIL_LABEL__", JSONObject.quote(context.getString(R.string.found_by_zajil)))
            WebViewCompat.addDocumentStartJavaScript(view, script, setOf("*"))
            WebViewCompat.addWebMessageListener(view, bridge, setOf("*")) { _, message, origin, isMainFrame, _ ->
                // Untrusted page data: only a report with this load's token, bounded and checked.
                val report = Finds.parseReport(message.data, token) ?: return@addWebMessageListener
                elements["$isMainFrame|$origin"] = report.second
                if (isMainFrame && report.first.isNotBlank()) _page.value = _page.value?.copy(title = report.first)
                publish()
            }
        }
        return view
    }

    private fun reset() {
        sitePage = null
        media.clear()
        elements.clear()
        _found.value = emptyList()
    }

    /** The page's address changed: layer 1 again, and what it asks for from now on. */
    private fun moved(url: String) {
        sitePage = if (VideoPages.isVideoPage(url)) Found.SitePage(url, _page.value?.title.orEmpty()) else null
        media.clear()
        publish()
    }

    /**
     * The list for the «Download» button. On a video site's page, the page
     * alone: yt-dlp reads it whole (its media requests are pieces). Else
     * the media files and main playlists the page asked for (a rendition
     * only when there is no main one), then its videos with an address of
     * their own, then the protected ones, which are not downloaded.
     */
    private fun publish() {
        sitePage?.let {
            _found.value = listOf(it)
            return
        }
        // A feed (TikTok's «For you», Instagram's, Facebook's): its videos
        // by their own posts' pages, which yt-dlp reads; the files a feed
        // plays are pieces. Only the page's own site's.
        val pageUrl = _page.value?.url
        val posts = elements.values.flatten().map { it.page }
            .filter { it.isNotEmpty() && pageUrl != null && Finds.sameSite(it, pageUrl) }
            .distinct()
        if (posts.isNotEmpty()) {
            _found.value = posts.take(Finds.MAX_VIDEOS).map { Found.SitePage(it, "") }
            return
        }
        val list = mutableListOf<Found>()
        val playlists = media.values.filter { it.kind != MediaKind.File }
        val mains = playlists.filterNot { Finds.isRendition(it.url) }
        list += (mains.ifEmpty { playlists.take(1) })
        val videos = elements.values.flatten()
        // A file the page's <video> plays: its length from there.
        list += media.values.filter { it.kind == MediaKind.File }.map { file ->
            val length = videos.firstOrNull { it.src.substringBefore('?') == file.url.substringBefore('?') }?.duration ?: 0
            if (length > 0) file.copy(duration = length) else file
        }
        val known = list.map { (it as Found.Media).url.substringBefore('?') }.toSet()
        videos.filter { it.src.isNotEmpty() && !it.protected && it.src.substringBefore('?') !in known }
            .distinctBy { it.src }
            .forEach { list += Found.Element(it.src, it.width, it.height, it.duration, protected = false) }
        videos.filter { it.protected }.take(1).forEach { list += Found.Element(null, it.width, it.height, it.duration, protected = true) }
        _found.value = list
    }

    private inner class Client : WebViewClient() {
        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
            // Only http(s): no intent:, file:, content: or another app's links.
            val scheme = request.url.scheme?.lowercase()
            return scheme != "http" && scheme != "https"
        }

        override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) {
            elements.clear()
            _page.value = PageState(url, loading = true, canGoBack = view.canGoBack())
            moved(url)
        }

        override fun doUpdateVisitedHistory(view: WebView, url: String, isReload: Boolean) {
            val before = _page.value?.url
            _page.value = _page.value?.copy(url = url, canGoBack = view.canGoBack()) ?: PageState(url)
            if (url != before) moved(url)
        }

        override fun onPageFinished(view: WebView, url: String) {
            _page.value = _page.value?.copy(loading = false, progress = 100, canGoBack = view.canGoBack())
            sitePage = sitePage?.copy(title = view.title.orEmpty())
            publish()
            if (_private.value || !url.startsWith("http")) return
            store.visited(Visit(view.title?.takeIf { it.isNotBlank() } ?: url, url))
            if (!CookieManager.getInstance().getCookie(url).isNullOrEmpty()) store.keepsCookies(url)
        }

        /** Layer 2: a media file or playlist the page asks for. Only looked at, never changed or fetched. */
        override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? {
            if (request.isForMainFrame) return null
            val url = request.url.toString()
            val kind = Finds.kindOf(url) ?: return null
            main.post {
                val key = url.substringBefore('?')
                if (key !in media && media.size < MAX_MEDIA) {
                    media[key] = Found.Media(url, kind)
                    publish()
                }
            }
            return null
        }

        /** A certificate Android does not trust: never shown, as everywhere in Zajil. */
        override fun onReceivedSslError(view: WebView, handler: SslErrorHandler, error: SslError) = handler.cancel()

        override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
            if (view == webView) close() else view.destroy()
            return true
        }
    }

    private inner class Chrome : WebChromeClient() {
        override fun onProgressChanged(view: WebView, newProgress: Int) {
            _page.value = _page.value?.copy(progress = newProgress, loading = newProgress < 100)
        }

        override fun onReceivedTitle(view: WebView, title: String?) {
            _page.value = _page.value?.copy(title = title.orEmpty())
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

        fun random(): String {
            val bytes = ByteArray(16)
            SecureRandom().nextBytes(bytes)
            return bytes.joinToString("") { "%02x".format(it) }
        }
    }
}
