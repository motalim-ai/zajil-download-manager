package io.github.motalim_ai.zajil.downloads

import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.ComponentName
import android.content.Context
import android.os.PersistableBundle
import io.github.motalim_ai.zajil.downloads
import java.time.ZonedDateTime

/**
 * Wakes the queues when the app is not running, with Android's own job
 * scheduler: at each scheduled queue's time, or when a usable network comes
 * back. Nothing stays alive in between (ADR-0012, section 3). The jobs
 * survive a restart of the phone.
 */
class QueueJobService : JobService() {
    override fun onStartJob(params: JobParameters): Boolean {
        val center = applicationContext.downloads
        if (params.jobId >= SCHEDULE_JOBS) center.openSchedule(params.extras.getLong(QUEUE, Queue.MAIN))
        center.pump()
        return false
    }

    override fun onStopJob(params: JobParameters): Boolean = false

    companion object {
        private const val NETWORK_JOB = 2

        /** A scheduled queue's job is this plus the queue's id (the main queue's, 1000, as before 0.3.0 was 1). */
        private const val SCHEDULE_JOBS = 1000
        private const val QUEUE = "queue"

        /** What the jobs should be, so that an unchanged plan is not scheduled again. */
        private var last: Pair<Map<Long, Long>, Int?>? = null

        /**
         * Schedules or cancels the jobs: each scheduled queue's next time if
         * it has downloads waiting for it, and a usable network if a queue
         * with waiting downloads waits for one ([waiting]: those queues).
         */
        fun plan(context: Context, waiting: Set<Long>, queues: List<Queue>, network: NetworkState) {
            val now = ZonedDateTime.now()
            val starts = queues.filter { it.id in waiting && it.rules.scheduled && !it.rules.open }
                .associate { it.id to QueuePlan.nextStart(now, it.rules.startAt, days = it.rules.days).toInstant().toEpochMilli() }
            val blocked = queues.filter { it.id in waiting && !network.allows(it.rules) }
            val networkType = when {
                blocked.isEmpty() -> null
                // Any network serves a queue that is not «Wi-Fi only».
                blocked.all { it.rules.wifiOnly } -> JobInfo.NETWORK_TYPE_UNMETERED
                else -> JobInfo.NETWORK_TYPE_ANY
            }
            synchronized(this) {
                if (last == starts to networkType) return
                last = starts to networkType
            }
            val jobs = context.getSystemService(JobScheduler::class.java)
            val service = ComponentName(context, QueueJobService::class.java)
            // The schedule job before 0.3.0 (id 1), and those of queues no longer scheduled.
            jobs.cancel(1)
            jobs.allPendingJobs.filter { it.id >= SCHEDULE_JOBS && (it.id - SCHEDULE_JOBS).toLong() !in starts }.forEach { jobs.cancel(it.id) }
            for ((queue, at) in starts) {
                jobs.schedule(
                    JobInfo.Builder(SCHEDULE_JOBS + queue.toInt(), service)
                        .setMinimumLatency((at - System.currentTimeMillis()).coerceAtLeast(0))
                        .setExtras(PersistableBundle().apply { putLong(QUEUE, queue) })
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
