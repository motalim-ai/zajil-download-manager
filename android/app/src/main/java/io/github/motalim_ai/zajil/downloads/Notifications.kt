package io.github.motalim_ai.zajil.downloads

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.text.format.Formatter
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import io.github.motalim_ai.zajil.MainActivity
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.downloads

/** The notifications Zajil posts: only about its downloads. */
object Notifications {
    private const val PROGRESS_CHANNEL = "progress"
    private const val STOPPED_CHANNEL = "stopped"
    const val PROGRESS_ID = 1
    private const val STOPPED_ID = 2

    fun createChannels(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                PROGRESS_CHANNEL,
                context.getString(R.string.channel_progress),
                NotificationManager.IMPORTANCE_LOW,
            ),
        )
        manager.createNotificationChannel(
            NotificationChannel(
                STOPPED_CHANNEL,
                context.getString(R.string.channel_stopped),
                NotificationManager.IMPORTANCE_DEFAULT,
            ),
        )
    }

    /** The foreground service's notification: the first running download. */
    fun progress(context: Context, list: List<Download>): Notification {
        val running = list.filter { it.status == Status.Running }
        val first = running.firstOrNull()
        val builder = NotificationCompat.Builder(context, PROGRESS_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_zajil)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .setCategory(NotificationCompat.CATEGORY_PROGRESS)
            .setContentIntent(openApp(context))
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
        if (first == null) {
            // Started downloads that wait for the network to come back.
            val waiting = list.count { it.status == Status.Queued }
            val center = context.downloads
            val wifi = list.filter { it.status == Status.Queued }.any { center.waitOf(it) == Wait.Wifi }
            return builder.setContentTitle(context.resources.getQuantityString(R.plurals.waiting, waiting, waiting))
                .setContentText(context.getString(if (wifi) R.string.wait_wifi else R.string.wait_network))
                .build()
        }
        val total = first.total
        builder.setContentTitle(FileNames.display(first.name))
            .setContentText(sizeText(context, first.done, total))
            .addAction(0, context.getString(R.string.pause), service(context, DownloadService.ACTION_PAUSE, first.id))
            .addAction(0, context.getString(R.string.cancel), service(context, DownloadService.ACTION_CANCEL, first.id))
        if (running.size > 1) {
            builder.setSubText(context.resources.getQuantityString(R.plurals.running, running.size, running.size))
        }
        if (total != null && total > 0) {
            val percent = (first.done * 100 / total).toInt().coerceIn(0, 100)
            builder.setProgress(100, percent, false)
        } else {
            builder.setProgress(0, 0, true)
        }
        return builder.build()
    }

    /**
     * Downloads that wait for the user: paused from the notification, or
     * stopped with the phone when Android does not let Zajil resume them by
     * itself (ADR-0012, section 3).
     */
    fun showStopped(context: Context, count: Int) {
        if (!NotificationManagerCompat.from(context).areNotificationsEnabled()) return
        val notification = NotificationCompat.Builder(context, STOPPED_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_zajil)
            .setContentTitle(context.resources.getQuantityString(R.plurals.stopped, count, count))
            .setContentText(context.getString(R.string.tap_to_resume))
            .setContentIntent(openApp(context, resume = true))
            .addAction(0, context.getString(R.string.resume), service(context, DownloadService.ACTION_RESUME_ALL, 0))
            .setAutoCancel(true)
            .build()
        try {
            NotificationManagerCompat.from(context).notify(STOPPED_ID, notification)
        } catch (_: SecurityException) {
            // Notifications not allowed: the app's list still shows them.
        }
    }

    /**
     * The queue's time has come while Zajil is in the background, and
     * Android does not let it start a download by itself: one tap does.
     */
    fun showScheduleDue(context: Context, count: Int) {
        if (!NotificationManagerCompat.from(context).areNotificationsEnabled()) return
        createChannels(context)
        val notification = NotificationCompat.Builder(context, STOPPED_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_zajil)
            .setContentTitle(context.getString(R.string.schedule_due))
            .setContentText(context.resources.getQuantityString(R.plurals.tap_to_start, count, count))
            .setContentIntent(openApp(context, resume = true))
            .addAction(0, context.getString(R.string.start_now), service(context, DownloadService.ACTION_RESUME_ALL, 0))
            .setAutoCancel(true)
            .build()
        try {
            NotificationManagerCompat.from(context).notify(STOPPED_ID, notification)
        } catch (_: SecurityException) {
            // Notifications not allowed: the app's list still shows them.
        }
    }

    fun hideStopped(context: Context) {
        NotificationManagerCompat.from(context).cancel(STOPPED_ID)
    }

    fun sizeText(context: Context, done: Long, total: Long?): String {
        val doneText = Formatter.formatShortFileSize(context, done)
        if (total == null || total <= 0) return doneText
        val percent = (done * 100 / total).coerceIn(0, 100)
        return context.getString(
            R.string.size_of_total,
            doneText,
            Formatter.formatShortFileSize(context, total),
            percent.toInt(),
        )
    }

    private fun openApp(context: Context, resume: Boolean = false): PendingIntent {
        val intent = Intent(context, MainActivity::class.java)
            .setFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            .putExtra(MainActivity.EXTRA_RESUME_ALL, resume)
        return PendingIntent.getActivity(
            context,
            if (resume) 1 else 0,
            intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

    private fun service(context: Context, action: String, id: Long): PendingIntent {
        val intent = Intent(context, DownloadService::class.java)
            .setAction(action)
            .putExtra(DownloadService.EXTRA_ID, id)
        return PendingIntent.getForegroundService(
            context,
            (action.hashCode() * 31 + id).toInt(),
            intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }
}
