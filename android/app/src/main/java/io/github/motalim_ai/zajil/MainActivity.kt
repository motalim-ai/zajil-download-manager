package io.github.motalim_ai.zajil

import android.Manifest
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.mutableStateOf
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import io.github.motalim_ai.zajil.downloads.Links
import io.github.motalim_ai.zajil.downloads.LocalNetwork
import io.github.motalim_ai.zajil.downloads.Notifications
import io.github.motalim_ai.zajil.ui.DownloadActions
import io.github.motalim_ai.zajil.ui.DownloadsScreen
import io.github.motalim_ai.zajil.ui.Offer
import io.github.motalim_ai.zajil.ui.UpdatePrefs
import io.github.motalim_ai.zajil.update.AppUpdate
import io.github.motalim_ai.zajil.ui.VideoSheet
import io.github.motalim_ai.zajil.ui.VideoSheetView
import io.github.motalim_ai.zajil.ui.ZajilTheme
import io.github.motalim_ai.zajil.video.VideoChoice
import io.github.motalim_ai.zajil.video.VideoPages
import io.github.motalim_ai.zajil.video.YtDlp
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity(), DownloadActions {
    private val askPermissions = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {}

    /** The download waiting for the local network permission, and when it was asked. */
    private var waitingForLocal = 0L
    private var askedLocalAt = 0L
    private val askLocal = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
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
    }

    /** A link offered from a share or the clipboard, until the user answers. */
    private val offer = mutableStateOf<Offer?>(null)

    /** A video link's format sheet, while it is open (A3). */
    private val videoSheet = mutableStateOf<VideoSheet?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        waitingForLocal = savedInstanceState?.getLong(WAITING_FOR_LOCAL) ?: 0
        enableEdgeToEdge()
        setContent {
            ZajilTheme {
                DownloadsScreen(actions = this, offer = offer.value, onOfferDone = { offer.value = null })
                videoSheet.value?.let { sheet ->
                    VideoSheetView(
                        sheet,
                        onDownload = { url, info, picked, choice ->
                            videoSheet.value = null
                            downloads.addVideo(url, info, picked, choice)
                            askMissingPermissions()
                        },
                        onDismiss = { videoSheet.value = null },
                    )
                }
            }
        }
        if (savedInstanceState == null) handle(intent)
        // Zajil's own update, at most once a day if the user leaves it on:
        // only the signed update file is asked for; the card offers it.
        if (savedInstanceState == null && UpdatePrefs.due(this)) {
            lifecycleScope.launch { AppUpdate.check(quiet = true) }
        }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putLong(WAITING_FOR_LOCAL, waitingForLocal)
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

    /**
     * Android lets an app read the clipboard only while it has the focus: a
     * link copied since Zajil last looked is offered, once (A2). Nothing is
     * read in the background.
     */
    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus && offer.value == null) checkClipboard()
    }

    private fun checkClipboard() {
        val clipboard = getSystemService(ClipboardManager::class.java) ?: return
        val description = clipboard.primaryClipDescription ?: return
        if (!description.hasMimeType(ClipDescription.MIMETYPE_TEXT_PLAIN) &&
            !description.hasMimeType(ClipDescription.MIMETYPE_TEXT_URILIST)
        ) {
            return
        }
        // The description says when it was copied, without reading the text.
        val prefs = getSharedPreferences("clipboard", MODE_PRIVATE)
        val copiedAt = description.timestamp
        if (copiedAt <= prefs.getLong(SEEN, 0)) return
        prefs.edit().putLong(SEEN, copiedAt).apply()
        val text = clipboard.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(this) ?: return
        val link = Links.first(text.toString()) ?: return
        if (downloads.list.value.any { it.url == link }) return
        offer.value = Offer(link, fromClipboard = true)
    }

    private fun handle(intent: Intent?) {
        when {
            intent == null -> {}
            intent.action == Intent.ACTION_SEND && intent.type?.startsWith("text/") == true -> {
                // «Share to Zajil»: the link is offered; nothing downloads
                // until the user taps «Download».
                val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString().orEmpty() +
                    " " + intent.getStringExtra(Intent.EXTRA_SUBJECT).orEmpty()
                offer.value = Offer(Links.first(text), fromClipboard = false)
                setIntent(Intent(this, MainActivity::class.java))
            }
            intent.getBooleanExtra(EXTRA_RESUME_ALL, false) -> {
                intent.removeExtra(EXTRA_RESUME_ALL)
                Notifications.hideStopped(this)
                downloads.resumeAll()
            }
        }
    }

    override fun add(url: String) {
        if (VideoPages.isVideoPage(url)) {
            readVideo(url)
            return
        }
        lifecycleScope.launch {
            val local = LocalNetwork.isLocal(url)
            val id = downloads.add(url, local)
            if (local && !LocalNetwork.isGranted(this@MainActivity)) {
                askLocalNetwork(id)
            } else {
                askMissingPermissions()
            }
        }
    }

    /**
     * A video page: yt-dlp reads it on the phone, and the sheet offers its
     * qualities and sounds. Nothing downloads until the user chooses.
     */
    private fun readVideo(url: String) {
        videoSheet.value = VideoSheet.Reading(url)
        lifecycleScope.launch {
            val read = YtDlp.read(this@MainActivity, url)
            if (videoSheet.value?.url != url) return@launch
            videoSheet.value = read.fold(
                onSuccess = { info ->
                    when {
                        info.live -> VideoSheet.Refused(url, R.string.video_live, null)
                        VideoChoice.sounds(info).isEmpty() && info.formats.any { it.hasDrm } ->
                            VideoSheet.Refused(url, R.string.video_drm, null)
                        VideoChoice.sounds(info).isEmpty() -> VideoSheet.Refused(url, R.string.video_nothing, null)
                        else -> VideoSheet.Ready(url, info)
                    }
                },
                onFailure = { e ->
                    val detail = e.message.orEmpty()
                    val message = when {
                        "confirm you" in detail && "bot" in detail -> R.string.video_bot_check
                        "DRM" in detail -> R.string.video_drm
                        else -> R.string.video_unreadable
                    }
                    VideoSheet.Refused(url, message, detail.takeIf { message == R.string.video_unreadable })
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
        askLocal.launch(LocalNetwork.PERMISSION)
    }

    /**
     * Notifications (Android 13 and later) for the progress, and on Android
     * 8 and 9 the storage permission for the public Download folder.
     */
    private fun askMissingPermissions() {
        val wanted = buildList {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) add(Manifest.permission.POST_NOTIFICATIONS)
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) add(Manifest.permission.WRITE_EXTERNAL_STORAGE)
        }.filter { ContextCompat.checkSelfPermission(this, it) != PackageManager.PERMISSION_GRANTED }
        if (wanted.isNotEmpty()) askPermissions.launch(wanted.toTypedArray())
    }

    companion object {
        const val EXTRA_RESUME_ALL = "resume_all"
        private const val WAITING_FOR_LOCAL = "waiting_for_local"
        private const val SEEN = "seen_at"
    }
}
