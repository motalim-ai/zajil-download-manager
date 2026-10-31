package io.github.motalim_ai.zajil.downloads

import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.ComponentName
import android.content.Context
import io.github.motalim_ai.zajil.downloads
import java.time.ZonedDateTime

/**
 * Wakes the queue when the app is not running, with Android's own job
 * scheduler: at the schedule's time, or when a usable network comes back.
 * Nothing stays alive in between (ADR-0012, section 3). Both jobs survive a
 * restart of the phone.
 */
class QueueJobService : JobService() {
    override fun onStartJob(params: JobParameters): Boolean {
        val center = applicationContext.downloads
        if (params.jobId == SCHEDULE_JOB) center.openSchedule()
        center.pump()
        return false
    }

    override fun onStopJob(params: JobParameters): Boolean = false

    companion object {
        private const val SCHEDULE_JOB = 1
        private const val NETWORK_JOB = 2

        /** What the jobs should be, so that an unchanged plan is not scheduled again. */
        private var last: Pair<Long?, Int?> = null to null

        /**
         * Schedules or cancels the two jobs: the schedule's next time if the
         * queue waits for it, and a usable network if the queue waits for one.
         */
        fun plan(context: Context, waiting: Boolean, rules: QueueRules, network: NetworkState) {
            val startAt = if (waiting && rules.scheduled && !rules.open) {
                QueuePlan.nextStart(ZonedDateTime.now(), rules.startAt).toInstant().toEpochMilli()
            } else {
                null
            }
            val networkType = when {
                !waiting || network.allows(rules) -> null
                rules.wifiOnly -> JobInfo.NETWORK_TYPE_UNMETERED
                else -> JobInfo.NETWORK_TYPE_ANY
            }
            synchronized(this) {
                if (last == startAt to networkType) return
                last = startAt to networkType
            }
            val jobs = context.getSystemService(JobScheduler::class.java)
            val service = ComponentName(context, QueueJobService::class.java)
            if (startAt == null) {
                jobs.cancel(SCHEDULE_JOB)
            } else {
                jobs.schedule(
                    JobInfo.Builder(SCHEDULE_JOB, service)
                        .setMinimumLatency((startAt - System.currentTimeMillis()).coerceAtLeast(0))
                        .setPersisted(true)
                        .build(),
                )
            }
            if (networkType == null) {
                jobs.cancel(NETWORK_JOB)
            } else {
                jobs.schedule(
                    JobInfo.Builder(NETWORK_JOB, service)
                        .setRequiredNetworkType(networkType)
                        .setPersisted(true)
                        .build(),
                )
            }
        }
    }
}
