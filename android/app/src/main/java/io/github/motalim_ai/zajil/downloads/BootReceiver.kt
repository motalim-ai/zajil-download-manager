package io.github.motalim_ai.zajil.downloads

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build
import io.github.motalim_ai.zajil.downloads

/**
 * After the phone starts: downloads that were running resume, from where
 * they stopped. From Android 15 a data transfer may not start here, so a
 * notification asks the user to tap instead (ADR-0012, section 3).
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return
        val center = context.downloads
        if (!center.hasInterrupted()) return
        Notifications.createChannels(context)
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.VANILLA_ICE_CREAM) {
            try {
                DownloadService.start(context, DownloadService.ACTION_RESUME_INTERRUPTED)
                return
            } catch (_: IllegalStateException) {
                // Not allowed after all: ask the user below.
            }
        }
        Notifications.showStopped(context, center.list.value.count { it.status == Status.Interrupted })
    }
}
