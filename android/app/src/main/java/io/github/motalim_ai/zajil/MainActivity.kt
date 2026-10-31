package io.github.motalim_ai.zajil

import android.Manifest
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
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import io.github.motalim_ai.zajil.downloads.DownloadService
import io.github.motalim_ai.zajil.downloads.LocalNetwork
import io.github.motalim_ai.zajil.downloads.Notifications
import io.github.motalim_ai.zajil.ui.DownloadsScreen
import io.github.motalim_ai.zajil.ui.ZajilTheme
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val askPermissions = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {}

    /** The download waiting for the local network permission, and when it was asked. */
    private var waitingForLocal = 0L
    private var askedLocalAt = 0L
    private val askLocal = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        val id = waitingForLocal
        waitingForLocal = 0
        if (granted) {
            resume(id)
        } else if (SystemClock.elapsedRealtime() - askedLocalAt < 500) {
            // Android answered without asking: the user said "don't ask
            // again". Only the app's settings page can change it now.
            startActivity(
                Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", packageName, null)),
            )
        }
        // Declined: the download stays, and says it needs the permission.
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        waitingForLocal = savedInstanceState?.getLong(WAITING_FOR_LOCAL) ?: 0
        enableEdgeToEdge()
        setContent {
            ZajilTheme {
                DownloadsScreen(
                    onAdd = ::add,
                    onPause = { downloads.pause(it) },
                    onResume = ::resumeAsked,
                    onRemove = { downloads.cancel(it) },
                )
            }
        }
        handle(intent)
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
        // What stopped with the app or the phone resumes once Zajil is open.
        if (downloads.hasInterrupted()) {
            Notifications.hideStopped(this)
            DownloadService.start(this, DownloadService.ACTION_RESUME_INTERRUPTED)
        }
    }

    private fun handle(intent: Intent?) {
        if (intent?.getBooleanExtra(EXTRA_RESUME_ALL, false) == true) {
            intent.removeExtra(EXTRA_RESUME_ALL)
            Notifications.hideStopped(this)
            DownloadService.start(this, DownloadService.ACTION_RESUME_ALL)
        }
    }

    private fun add(url: String) {
        lifecycleScope.launch {
            val local = LocalNetwork.isLocal(url)
            val id = downloads.add(url, local)
            if (local && !LocalNetwork.isGranted(this@MainActivity)) {
                askLocalNetwork(id)
            } else {
                askMissingPermissions()
                DownloadService.start(this@MainActivity, DownloadService.ACTION_SHOW)
            }
        }
    }

    /** «Resume» or «Allow»: the local network permission first, if needed. */
    private fun resumeAsked(id: Long) {
        val download = downloads.find(id) ?: return
        if (download.local && !LocalNetwork.isGranted(this)) askLocalNetwork(id) else resume(id)
    }

    private fun askLocalNetwork(id: Long) {
        waitingForLocal = id
        askedLocalAt = SystemClock.elapsedRealtime()
        askLocal.launch(LocalNetwork.PERMISSION)
    }

    private fun resume(id: Long) {
        downloads.resume(id)
        DownloadService.start(this, DownloadService.ACTION_SHOW)
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
    }
}
