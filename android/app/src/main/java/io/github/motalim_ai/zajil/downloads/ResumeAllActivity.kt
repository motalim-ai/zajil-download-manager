package io.github.motalim_ai.zajil.downloads

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import io.github.motalim_ai.zajil.MainActivity
import io.github.motalim_ai.zajil.downloads

/**
 * The «resume» notification's tap: resumes every download waiting for the
 * user, then opens Zajil. Not exported, so only Zajil's own notification
 * (its PendingIntent) reaches it; no other app can resume downloads.
 */
class ResumeAllActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Notifications.hideStopped(this)
        downloads.resumeAll()
        startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP))
        finish()
    }
}
