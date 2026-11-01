package io.github.motalim_ai.zajil

import android.content.Intent
import android.os.Bundle
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import io.github.motalim_ai.zajil.browser.BrowserController
import io.github.motalim_ai.zajil.browser.Finds
import io.github.motalim_ai.zajil.browser.Found
import io.github.motalim_ai.zajil.browser.MediaKind
import io.github.motalim_ai.zajil.downloads.CopiedLinks
import io.github.motalim_ai.zajil.downloads.Links
import io.github.motalim_ai.zajil.downloads.Notifications
import io.github.motalim_ai.zajil.ui.Offer
import io.github.motalim_ai.zajil.ui.Source
import io.github.motalim_ai.zajil.ui.UpdatePrefs
import io.github.motalim_ai.zajil.ui.ZajilScreen
import io.github.motalim_ai.zajil.ui.ZajilTheme
import io.github.motalim_ai.zajil.ui.isDark
import io.github.motalim_ai.zajil.update.AppUpdate
import io.github.motalim_ai.zajil.video.VideoPages
import kotlinx.coroutines.launch

class MainActivity : ZajilActivity() {
    /** The in-app browser (A6): its WebView lives as long as this window. */
    private val browser by lazy { BrowserController(this) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // A page's own download: offered in the sheet, never started by the page.
        lifecycleScope.launch {
            browser.requests.collect { url -> offer.value = Offer(url, Source.Browser, browser.linkFor(url)) }
        }
        enableEdgeToEdge()
        setContent {
            val prefs by downloads.prefs.prefs.collectAsStateWithLifecycle()
            val dark = isDark(prefs.look)
            // The status bar's icons follow the app's look, which may not be the system's.
            LaunchedEffect(dark) {
                val style = if (dark) {
                    SystemBarStyle.dark(android.graphics.Color.TRANSPARENT)
                } else {
                    SystemBarStyle.light(android.graphics.Color.TRANSPARENT, android.graphics.Color.TRANSPARENT)
                }
                enableEdgeToEdge(statusBarStyle = style, navigationBarStyle = style)
            }
            ZajilTheme(prefs.look) {
                ZajilScreen(
                    actions = this,
                    onNewLink = { offer.value = Offer(null, Source.Typed) },
                    browser = browser,
                    onFound = ::download,
                )
                Sheets()
            }
        }
        if (savedInstanceState == null) handle(intent)
        // Zajil's own update, at most once a day if the user leaves it on:
        // only the signed update file is asked for; the card offers it.
        if (savedInstanceState == null && UpdatePrefs.due(this)) {
            lifecycleScope.launch { AppUpdate.check(quiet = true) }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    override fun onStart() {
        super.onStart()
        // What stopped with the app or the phone resumes once Zajil is open,
        // and a queue that could not start in the background starts now.
        Notifications.hideStopped(this)
        downloads.resumeInterrupted()
    }

    override fun onResume() {
        super.onResume()
        browser.onResume()
        lookAtClipboardSoon()
    }

    override fun onPause() {
        browser.onPause()
        super.onPause()
    }

    override fun onDestroy() {
        browser.destroy()
        super.onDestroy()
    }

    /**
     * A video the browser found, after the user's tap (A6): a video site's
     * page and a playlist go to yt-dlp's sheet, a file to the new-download
     * sheet; each with the page and, for the page's own site, its cookies.
     */
    private fun download(found: Found) {
        when (found) {
            is Found.SitePage -> readVideo(found.url, browser.linkFor(found.url))
            is Found.Media -> when (found.kind) {
                MediaKind.Hls -> readVideo(found.url, browser.linkFor(found.url), browser.page.value?.title)
                MediaKind.File -> offer.value = Offer(found.url, Source.Browser, browser.linkFor(found.url))
                MediaKind.Dash -> {}
            }
            is Found.Element -> found.src?.let { src ->
                if (Finds.kindOf(src) == MediaKind.Hls) {
                    readVideo(src, browser.linkFor(src), browser.page.value?.title)
                } else {
                    offer.value = Offer(src, Source.Browser, browser.linkFor(src))
                }
            }
        }
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) lookAtClipboardSoon()
    }

    /**
     * Android lets an app read the clipboard only while its window has the
     * focus: a link copied since Zajil last looked is offered, once, a
     * moment after the window has it (some phones answer nothing at the
     * instant itself: then once more a little later). Nothing is read in the
     * background, nor while a sheet is open.
     */
    private fun lookAtClipboardSoon() {
        val view = window.decorView
        view.removeCallbacks(lookAtClipboard)
        view.removeCallbacks(lookAgain)
        view.postDelayed(lookAtClipboard, 250)
    }

    private val lookAgain = Runnable { lookAtClipboard(retry = false) }
    private val lookAtClipboard = Runnable { lookAtClipboard(retry = true) }

    private fun lookAtClipboard(retry: Boolean) {
        if (!hasWindowFocus() || offer.value != null || videoSheet.value != null) return
        when (val copied = CopiedLinks.read(this, onlyNew = true) { link -> downloads.list.value.any { it.url == link } }) {
            CopiedLinks.Copied.Unavailable -> if (retry) window.decorView.postDelayed(lookAgain, 750)
            is CopiedLinks.Copied.Link -> offer.value = Offer(copied.url, Source.Clipboard)
            CopiedLinks.Copied.Seen, CopiedLinks.Copied.NoLink -> {}
        }
    }

    private fun handle(intent: Intent?) {
        when {
            intent == null -> {}
            intent.action == Intent.ACTION_SEND && intent.type?.startsWith("text/") == true -> {
                // «Share to Zajil»: the link is offered; nothing downloads
                // until the user taps «Start». A video page opens its
                // format sheet, which asks the same.
                val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString().orEmpty() +
                    " " + intent.getStringExtra(Intent.EXTRA_SUBJECT).orEmpty()
                val link = Links.first(text)
                if (link != null && VideoPages.isVideoPage(link)) {
                    offer.value = null
                    readVideo(link)
                } else {
                    offer.value = Offer(link, Source.Shared)
                }
                setIntent(Intent(this, MainActivity::class.java))
            }
            intent.getBooleanExtra(EXTRA_RESUME_ALL, false) -> {
                intent.removeExtra(EXTRA_RESUME_ALL)
                Notifications.hideStopped(this)
                downloads.resumeAll()
            }
        }
    }

    companion object {
        const val EXTRA_RESUME_ALL = "resume_all"
    }
}
