package io.github.motalim_ai.zajil

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.provider.Settings
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import io.github.motalim_ai.zajil.downloads.BrowserLink
import io.github.motalim_ai.zajil.downloads.LocalNetwork
import io.github.motalim_ai.zajil.ui.AddSheet
import io.github.motalim_ai.zajil.ui.DownloadActions
import io.github.motalim_ai.zajil.ui.Offer
import io.github.motalim_ai.zajil.ui.StartWhen
import io.github.motalim_ai.zajil.ui.VideoSheet
import io.github.motalim_ai.zajil.ui.VideoSheetView
import io.github.motalim_ai.zajil.video.VideoChoice
import io.github.motalim_ai.zajil.video.VideoPages
import io.github.motalim_ai.zajil.video.VideoProblems
import io.github.motalim_ai.zajil.video.YtDlp
import kotlinx.coroutines.launch

/**
 * What both Zajil windows do with a link: the main screen, and the
 * transparent one the Quick Settings tile opens. The new-download and video
 * sheets, adding the download, and the permissions it needs.
 */
abstract class ZajilActivity : ComponentActivity(), DownloadActions {
    /** Permission questions on screen: the window waits for them before it is done. */
    private var asking = 0

    private val askPermissions = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        asking--
        settledIfDone()
    }

    /** The download waiting for the local network permission, and when it was asked. */
    private var waitingForLocal = 0L
    private var askedLocalAt = 0L
    private val askLocal = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        asking--
        val id = waitingForLocal
        waitingForLocal = 0
        if (granted) {
            downloads.resume(id)
        } else if (SystemClock.elapsedRealtime() - askedLocalAt < 500) {
            // Android answered without asking: the user said "don't ask
            // again". Only the app's settings page can change it now.
            startActivity(
                Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", packageName, null)),
            )
        }
        // Declined: the download stays, and says it needs the permission.
        settledIfDone()
    }

    /** A link offered from a share, the clipboard or the tile, until the user answers. */
    protected val offer = mutableStateOf<Offer?>(null)

    /** A video link's format sheet, while it is open (A3). */
    protected val videoSheet = mutableStateOf<VideoSheet?>(null)

    /** The browser's page and cookies for the video the sheet is about (A6), in memory only. */
    private var videoLink: BrowserLink? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        waitingForLocal = savedInstanceState?.getLong(WAITING_FOR_LOCAL) ?: 0
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putLong(WAITING_FOR_LOCAL, waitingForLocal)
    }

    /** Nothing is open or asked any more: the tile's window closes then. */
    protected open fun settled() {}

    private fun settledIfDone() {
        if (asking <= 0 && offer.value == null && videoSheet.value == null) settled()
    }

    /** The sheets over the window, while one is open. */
    @Composable
    protected fun Sheets() {
        offer.value?.let { asked ->
            AddSheet(
                asked,
                onStart = { url, name, start ->
                    offer.value = null
                    add(url, name, start, asked.link)
                },
                onDismiss = {
                    offer.value = null
                    settledIfDone()
                },
            )
        }
        videoSheet.value?.let { sheet ->
            VideoSheetView(
                sheet,
                onDownload = { url, info, picked, choice ->
                    videoSheet.value = null
                    downloads.addVideo(url, info, picked, choice, videoLink)
                    added()
                    askMissingPermissions()
                    settledIfDone()
                },
                onDismiss = {
                    videoSheet.value = null
                    settledIfDone()
                },
                onBrowse = if (browses) {
                    { url ->
                        videoSheet.value = null
                        openInBrowser(url)
                    }
                } else {
                    null
                },
            )
        }
    }

    /** Whether this window has Zajil's browser, to sign in on a site there. */
    protected open val browses: Boolean = false

    /** Opens [url] in Zajil's browser, where the user can sign in on its site. */
    protected open fun openInBrowser(url: String) {}

    /** A download was added: the tile's window says so. */
    protected open fun added() {}

    override fun add(url: String, name: String?, start: StartWhen, browser: BrowserLink?) {
        if (VideoPages.isVideoPage(url)) {
            readVideo(url, browser)
            return
        }
        lifecycleScope.launch {
            val local = LocalNetwork.isLocal(url)
            val id = downloads.add(url, local, name, later = start == StartWhen.Later, browser = browser)
            added()
            when {
                local && !LocalNetwork.isGranted(this@ZajilActivity) -> if (start != StartWhen.Later) askLocalNetwork(id)
                else -> {
                    if (start == StartWhen.Now) downloads.startNow(id)
                    if (start != StartWhen.Later) askMissingPermissions()
                }
            }
            settledIfDone()
        }
    }

    /**
     * A video page: yt-dlp reads it on the phone, and the sheet offers its
     * qualities and sounds. Nothing downloads until the user chooses.
     */
    protected fun readVideo(url: String, browser: BrowserLink? = null, pageTitle: String? = null) {
        videoLink = browser
        videoSheet.value = VideoSheet.Reading(url)
        lifecycleScope.launch {
            val read = YtDlp.read(this@ZajilActivity, url, browser?.cookies, browser?.page)
            if (videoSheet.value?.url != url) return@launch
            videoSheet.value = read.map { info ->
                // A playlist found in the browser has only its file's name
                // («master»): the page's title says what it is.
                val bare = url.substringBefore('?').substringAfterLast('/').substringBeforeLast('.')
                if (pageTitle.isNullOrBlank() || (info.title.isNotBlank() && info.title != bare)) info else info.copy(title = pageTitle)
            }.fold(
                onSuccess = { info ->
                    when {
                        info.live -> VideoSheet.Refused(url, R.string.video_live)
                        VideoChoice.sounds(info).isEmpty() && info.formats.any { it.hasDrm } ->
                            VideoSheet.Refused(url, R.string.video_drm)
                        VideoChoice.sounds(info).isEmpty() -> VideoSheet.Refused(url, R.string.video_nothing)
                        else -> VideoSheet.Ready(url, info)
                    }
                },
                onFailure = { e ->
                    val detail = e.message.orEmpty()
                    // yt-dlp's own words go to the log only, without any address.
                    Log.w("ZajilVideo", VideoProblems.forLog(detail))
                    val problem = VideoProblems.of(detail, url)
                    VideoSheet.Refused(url, problem.message, problem.signIn)
                },
            )
        }
    }

    override fun pause(id: Long) = downloads.pause(id)

    /** «Resume» or «Allow»: the local network permission first, if needed. */
    override fun resume(id: Long) {
        val download = downloads.find(id) ?: return
        if (download.local && !LocalNetwork.isGranted(this)) askLocalNetwork(id) else downloads.resume(id)
    }

    override fun startNow(id: Long) = downloads.startNow(id)

    override fun moveUp(id: Long) = downloads.moveUp(id)

    override fun remove(id: Long) = downloads.cancel(id)

    private fun askLocalNetwork(id: Long) {
        waitingForLocal = id
        askedLocalAt = SystemClock.elapsedRealtime()
        asking++
        askLocal.launch(LocalNetwork.PERMISSION)
    }

    /**
     * Notifications (Android 13 and later) for the progress, and on Android
     * 8 and 9 the storage permission for the public Download folder.
     */
    protected fun askMissingPermissions() {
        val wanted = buildList {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) add(Manifest.permission.POST_NOTIFICATIONS)
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) add(Manifest.permission.WRITE_EXTERNAL_STORAGE)
        }.filter { ContextCompat.checkSelfPermission(this, it) != PackageManager.PERMISSION_GRANTED }
        if (wanted.isNotEmpty()) {
            asking++
            askPermissions.launch(wanted.toTypedArray())
        }
    }

    private companion object {
        const val WAITING_FOR_LOCAL = "waiting_for_local"
    }
}
