package io.github.motalim_ai.zajil

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import io.github.motalim_ai.zajil.downloads.DownloadService
import io.github.motalim_ai.zajil.downloads.Notifications
import io.github.motalim_ai.zajil.ui.DownloadsScreen
import io.github.motalim_ai.zajil.ui.ZajilTheme

class MainActivity : ComponentActivity() {
    private val askPermissions = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {}

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            ZajilTheme {
                DownloadsScreen(
                    onAdd = ::add,
                    onPause = { downloads.pause(it) },
                    onResume = ::resume,
                    onRemove = { downloads.cancel(it) },
                )
            }
        }
        handle(intent)
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
        askMissingPermissions()
        downloads.add(url)
        DownloadService.start(this, DownloadService.ACTION_SHOW)
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
    }
}
