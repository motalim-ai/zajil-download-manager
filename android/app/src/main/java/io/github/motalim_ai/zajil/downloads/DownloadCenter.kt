package io.github.motalim_ai.zajil.downloads

import android.content.Context
import android.os.Environment
import android.util.AtomicFile
import io.github.motalim_ai.zajil.engine.Downloads
import io.github.motalim_ai.zajil.engine.Listener
import io.github.motalim_ai.zajil.engine.Problem
import java.io.File
import java.io.FileNotFoundException
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * The app's downloads: the list the screen and the notification show, the
 * queue that decides which of them run (A2), the engine that runs them, and
 * the file that keeps the list between runs. One for the whole process
 * ([io.github.motalim_ai.zajil.ZajilApp]).
 */
class DownloadCenter(private val context: Context) {
    private val listFile = AtomicFile(File(context.filesDir, "downloads.json"))
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /**
     * Saves the list one at a time, in order, and never on the caller's
     * thread: the engine reports from its own async threads, which must not
     * wait on the disk (AGENTS.md, section 10.6).
     */
    private val writer = Dispatchers.IO.limitedParallelism(1)

    /** Runs the queue one decision at a time, off the engine's threads too. */
    private val pumper = Dispatchers.IO.limitedParallelism(1)

    private val _list = MutableStateFlow(load())
    val list: StateFlow<List<Download>> = _list.asStateFlow()
    private val engine = Downloads(EngineListener())
    val settings = QueueSettings(context)
    private val network = NetworkWatch(context)
    val networkState: StateFlow<NetworkState> = network.state

    /** Cancelled while running: removed once the engine has stopped. */
    private val cancelled = ConcurrentHashMap.newKeySet<Long>()

    /** Stopped because the network went: back to the queue once stopped. */
    private val requeued = ConcurrentHashMap.newKeySet<Long>()

    /**
     * Started, then stopped by the network: they wait for it with the
     * service still in the foreground, because Android would not let it
     * start again from the background when the network returns. The
     * notification says what they wait for.
     */
    private val heldForNetwork = ConcurrentHashMap.newKeySet<Long>()

    /** Stopped because Android did not let the service run: interrupted. */
    private val interrupted = ConcurrentHashMap.newKeySet<Long>()

    /**
     * Where unfinished downloads live: the app's own folder on the shared
     * storage, which needs no permission (ADR-0012, section 4).
     */
    private val workDir: File
        get() = context.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS)
            ?: File(context.filesDir, "downloads")

    /**
     * Each download in a folder of its own, so that two at once from the same
     * link never make the engine rename one ("name (1)"): the only renaming
     * is the public folder's, once.
     */
    private fun folderOf(id: Long) = File(workDir, id.toString())

    init {
        // A download complete before the app stopped, not yet published.
        _list.value.filter { it.status == Status.Publishing }.forEach(::publish)
        // The network or the rules changed: stop what may not run, start
        // what may. A network that goes for a moment (a hand-over between
        // Wi-Fi and mobile data) is given a second and a half first.
        scope.launch {
            combine(network.state, settings.rules, ::Pair).collectLatest { (state, rules) ->
                if (!state.allows(rules)) {
                    delay(1_500)
                    _list.value.filter { it.status == Status.Running }.forEach {
                        requeued += it.id
                        engine.pause(it.id)
                    }
                }
                pump()
            }
        }
    }

    /**
     * Adds [url] to the queue, unless it is on the local network ([local])
     * and Zajil may not go there yet: then it waits for the permission.
     */
    fun add(url: String, local: Boolean): Long {
        val blocked = local && !LocalNetwork.isGranted(context)
        var id = 0L
        change { list ->
            id = (list.maxOfOrNull { it.id } ?: 0) + 1
            val order = (list.maxOfOrNull { it.order } ?: 0) + 1
            val status = if (blocked) Status.NeedsLocalNetwork else Status.Queued
            list + Download(id = id, url = url, status = status, local = local, order = order)
        }
        pump()
        return id
    }

    fun pause(id: Long) {
        heldForNetwork -= id
        if (find(id)?.status == Status.Queued) {
            edit(id) { it.copy(status = Status.Paused) }
            pump()
        } else {
            engine.pause(id)
        }
    }

    /** Back into the queue, at its old place. */
    fun resume(id: Long) {
        enqueue(id)
        pump()
    }

    /** Starts it now, whatever the queue's limit or schedule. */
    fun startNow(id: Long) {
        if (find(id)?.status == Status.Queued || enqueue(id)) {
            scope.launch(pumper) { if (startAll(listOf(id))) planJobs() }
        }
    }

    /** One place earlier in the queue. */
    fun moveUp(id: Long) {
        change { QueuePlan.movedUp(it, id) }
    }

    /** Resumes what was running when the app or the phone stopped. */
    fun resumeInterrupted() {
        _list.value.filter { it.status == Status.Interrupted }.forEach { enqueue(it.id) }
        pump()
    }

    fun resumeAll() {
        _list.value.filter { it.status in resumable }.forEach { enqueue(it.id) }
        pump()
    }

    /** Android's time for data transfers is up: everything stops, queue included. */
    fun pauseAll() {
        change { list -> list.map { if (it.status == Status.Queued) it.copy(status = Status.Paused) else it } }
        _list.value.filter { it.status == Status.Running }.forEach { engine.pause(it.id) }
    }

    /**
     * The service could not come to the foreground (after a restart of the
     * phone, from Android 15): what started stops as interrupted, and
     * resumes once the user opens Zajil.
     */
    fun stopWithoutService() {
        _list.value.filter { it.status == Status.Running }.forEach {
            interrupted += it.id
            engine.pause(it.id)
        }
    }

    /** The schedule's time has come: the queue runs until it is empty. */
    fun openSchedule() {
        settings.update { if (it.scheduled) it.copy(open = true) else it }
    }

    /** Stops it if it runs, then removes it with its files. */
    fun cancel(id: Long) {
        if (engine.isRunning(id)) {
            cancelled += id
            engine.pause(id)
        } else {
            remove(id)
        }
    }

    /** Removes it from the list; an unfinished download's files go too. */
    fun remove(id: Long) {
        val download = find(id) ?: return
        if (engine.isRunning(id)) return
        if (download.status != Status.Done) {
            // Our own folder: deleting removes a link itself, never its target.
            download.file?.let { File("$it.zajil-part").delete() }
            download.stateFile?.let { File(it).delete() }
        }
        // Only if empty: never anything else that might be there.
        folderOf(id).delete()
        heldForNetwork -= id
        change { list -> list.filterNot { it.id == id } }
        pump()
    }

    fun hasInterrupted() = _list.value.any { it.status == Status.Interrupted }

    /** Whether the service stays in the foreground for [list]. */
    fun keepsService(list: List<Download>): Boolean =
        list.any { it.status == Status.Running || (it.status == Status.Queued && it.id in heldForNetwork) }

    fun find(id: Long) = _list.value.firstOrNull { it.id == id }

    /** Why a queued download waits, for the screen. */
    fun waitOf(): Wait = QueuePlan.waitOf(settings.rules.value, network.state.value)

    /**
     * Starts what the queue allows now, then plans the jobs that wake it
     * later. [then] runs once done (a job's end).
     */
    fun pump(then: (() -> Unit)? = null) {
        scope.launch(pumper) {
            val rules = settings.rules.value
            val ids = QueuePlan.toStart(_list.value, rules, network.state.value)
            if (ids.isNotEmpty()) startAll(ids)
            // A scheduled queue that has run empty waits for its next time.
            if (rules.scheduled && rules.open && _list.value.none { it.status in active }) {
                settings.update { it.copy(open = false) }
            }
            planJobs()
            then?.invoke()
        }
    }

    /**
     * Starts [ids] with the service in the foreground; false if Android
     * would not let it come there now (the app is in the background): they
     * stay queued, and a notification lets the user start them.
     */
    private fun startAll(ids: List<Long>): Boolean {
        heldForNetwork -= ids.toSet()
        change { list -> list.map { if (it.id in ids) it.copy(status = Status.Running, problem = null, detail = null) else it } }
        if (!DownloadService.ensure(context)) {
            change { list -> list.map { if (it.id in ids) it.copy(status = Status.Queued) else it } }
            val waiting = QueuePlan.queued(_list.value).size
            val rules = settings.rules.value
            if (rules.scheduled && rules.open) {
                Notifications.showScheduleDue(context, waiting)
            } else {
                Notifications.showStopped(context, waiting)
            }
            return false
        }
        ids.mapNotNull(::find).forEach { download ->
            val stateFile = download.stateFile
            if (stateFile != null && File(stateFile).exists()) {
                engine.resume(download.id, stateFile)
            } else {
                // No data had been written: start again from the link.
                engine.start(download.id, download.url, folderOf(download.id).path)
            }
        }
        return true
    }

    /** Queued again; false if it cannot be (running, done, or no permission). */
    private fun enqueue(id: Long): Boolean {
        val download = find(id) ?: return false
        if (download.status !in resumable || engine.isRunning(id)) return false
        if (download.local && !LocalNetwork.isGranted(context)) {
            // Revoked since, or never given: never a connection that would
            // only time out. Also in the background (after a restart).
            edit(id) { it.copy(status = Status.NeedsLocalNetwork) }
            return false
        }
        edit(id) { it.copy(status = Status.Queued, problem = null, detail = null) }
        return true
    }

    private fun planJobs() {
        QueueJobService.plan(
            context,
            waiting = _list.value.any { it.status == Status.Queued },
            rules = settings.rules.value,
            network = network.state.value,
        )
    }

    private fun publish(download: Download) {
        scope.launch {
            val file = download.file?.let(::File)
            val published = file?.let { Publisher.publish(context, it) }
            if (published != null) folderOf(download.id).delete()
            edit(download.id) {
                if (published != null) {
                    it.copy(status = Status.Done, uri = published.uri.toString(), shownName = published.name)
                } else {
                    it.copy(status = Status.Failed, problem = Problem.OTHER, stateFile = null)
                }
            }
        }
    }

    /** Where a download goes once the engine has stopped it. */
    private fun stoppedAs(id: Long, otherwise: Status): Status = when {
        interrupted.remove(id) -> Status.Interrupted
        requeued.remove(id) -> Status.Queued.also { heldForNetwork += id }
        else -> otherwise.also { if (it == Status.Queued) heldForNetwork += id }
    }

    private inner class EngineListener : Listener {
        override fun started(id: Long, file: String, stateFile: String, total: ULong?, from: ULong) {
            edit(id) {
                it.copy(file = file, stateFile = stateFile, total = total?.toLong(), done = from.toLong())
            }
        }

        override fun progress(id: Long, done: ULong, total: ULong?) {
            // Not saved: the state file is what resuming trusts.
            _list.update { list ->
                list.map { if (it.id == id) it.copy(done = done.toLong(), total = total?.toLong()) else it }
            }
        }

        override fun finished(id: Long, file: String, bytes: ULong) {
            requeued.remove(id)
            interrupted.remove(id)
            edit(id) {
                it.copy(status = Status.Publishing, file = file, done = bytes.toLong(), total = bytes.toLong())
            }
            find(id)?.let(::publish)
            pump()
        }

        override fun paused(id: Long, stateFile: String?) {
            val status = stoppedAs(id, Status.Paused)
            edit(id) { it.copy(status = status, stateFile = stateFile ?: it.stateFile) }
            if (cancelled.remove(id)) remove(id) else pump()
        }

        override fun failed(id: Long, problem: Problem, detail: String, stateFile: String?) {
            // The connection failed because the network went: it waits in
            // the queue for the network, instead of failing.
            val offline = problem == Problem.NETWORK && !network.state.value.allows(settings.rules.value)
            val status = stoppedAs(id, if (offline) Status.Queued else Status.Failed)
            edit(id) {
                it.copy(
                    status = status,
                    problem = problem.takeIf { status == Status.Failed },
                    detail = detail.takeIf { status == Status.Failed },
                    stateFile = stateFile ?: it.stateFile,
                )
            }
            if (cancelled.remove(id)) remove(id) else pump()
        }
    }

    private fun edit(id: Long, how: (Download) -> Download) {
        change { list -> list.map { if (it.id == id) how(it) else it } }
    }

    private fun change(how: (List<Download>) -> List<Download>) {
        _list.update(how)
        scope.launch(writer) { save(_list.value) }
    }

    /** A failed save (a full disk) keeps the last good file; the next change tries again. */
    private fun save(list: List<Download>) {
        val out = try {
            listFile.startWrite()
        } catch (_: java.io.IOException) {
            return
        }
        try {
            out.write(Download.listToJson(list).toByteArray())
            listFile.finishWrite(out)
        } catch (_: java.io.IOException) {
            listFile.failWrite(out)
        }
    }

    /** What was running when the process ended is now interrupted. */
    private fun load(): List<Download> = try {
        Download.listFromJson(String(listFile.readFully())).map {
            if (it.status == Status.Running) it.copy(status = Status.Interrupted) else it
        }
    } catch (_: FileNotFoundException) {
        emptyList()
    }

    private companion object {
        val resumable = setOf(Status.Interrupted, Status.Paused, Status.Failed, Status.NeedsLocalNetwork)
        val active = setOf(Status.Queued, Status.Running)
    }
}
