package io.github.motalim_ai.zajil.downloads

import android.app.ForegroundServiceStartNotAllowedException
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import io.github.motalim_ai.zajil.downloads
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/**
 * Keeps downloads going while the app is not on screen, with a notification
 * that shows them (ADR-0012, section 3). Runs only while a download runs:
 * no other way of keeping Zajil alive.
 */
class DownloadService : LifecycleService() {
    private var watching: Job? = null

    /** When the notification last changed: Android drops faster updates. */
    private var shownAt = 0L

    /** Paused from the notification, or stopped by Android's time limit. */
    private var stoppedOutsideApp = false

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        Notifications.createChannels(this)
        if (!goForeground()) {
            // Android refused (after a restart of the phone, from Android
            // 15): the user resumes them from a notification instead.
            Notifications.showStopped(this, downloads.list.value.count { it.status == Status.Interrupted })
            stopSelf()
            return START_NOT_STICKY
        }
        val id = intent?.getLongExtra(EXTRA_ID, 0) ?: 0
        when (intent?.action) {
            ACTION_PAUSE -> {
                stoppedOutsideApp = true
                downloads.pause(id)
            }
            ACTION_CANCEL -> downloads.cancel(id)
            ACTION_RESUME_ALL -> {
                Notifications.hideStopped(this)
                downloads.resumeAll()
            }
            ACTION_SHOW -> {}
            // Restarted by Android after the process was killed (no intent),
            // or after the phone started.
            else -> downloads.resumeInterrupted()
        }
        if (watching == null) {
            watching = lifecycleScope.launch { downloads.list.collect(::show) }
        }
        return START_STICKY
    }

    private fun goForeground(): Boolean = try {
        ServiceCompat.startForeground(
            this,
            Notifications.PROGRESS_ID,
            Notifications.progress(this, downloads.list.value),
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC else 0,
        )
        true
    } catch (e: Exception) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && e is ForegroundServiceStartNotAllowedException) {
            false
        } else {
            throw e
        }
    }

    private fun show(list: List<Download>) {
        if (list.none { it.status == Status.Running }) {
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
            val waiting = list.count { it.status == Status.Paused || it.status == Status.Interrupted }
            if (stoppedOutsideApp && waiting > 0) Notifications.showStopped(this, waiting)
            stopSelf()
            return
        }
        val now = android.os.SystemClock.elapsedRealtime()
        if (now - shownAt < 500) return
        shownAt = now
        try {
            NotificationManagerCompat.from(this).notify(Notifications.PROGRESS_ID, Notifications.progress(this, list))
        } catch (_: SecurityException) {
            // Notifications not allowed: the download goes on regardless.
        }
    }

    /** Android 15 and later: data transfers get six hours a day. */
    override fun onTimeout(startId: Int, fgsType: Int) {
        stoppedOutsideApp = true
        downloads.pauseAll()
    }

    companion object {
        const val ACTION_SHOW = "io.github.motalim_ai.zajil.SHOW"
        const val ACTION_PAUSE = "io.github.motalim_ai.zajil.PAUSE"
        const val ACTION_CANCEL = "io.github.motalim_ai.zajil.CANCEL"
        const val ACTION_RESUME_ALL = "io.github.motalim_ai.zajil.RESUME_ALL"
        const val ACTION_RESUME_INTERRUPTED = "io.github.motalim_ai.zajil.RESUME_INTERRUPTED"
        const val EXTRA_ID = "id"

        fun start(context: Context, action: String) {
            ContextCompat.startForegroundService(
                context,
                Intent(context, DownloadService::class.java).setAction(action),
            )
        }
    }
}
