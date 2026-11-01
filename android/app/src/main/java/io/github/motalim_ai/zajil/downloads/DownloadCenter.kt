package io.github.motalim_ai.zajil.downloads

import android.content.Context
import android.os.Environment
import android.util.AtomicFile
import io.github.motalim_ai.zajil.engine.Downloads
import io.github.motalim_ai.zajil.engine.EngineException
import io.github.motalim_ai.zajil.engine.LinkInfo
import io.github.motalim_ai.zajil.engine.Listener
import io.github.motalim_ai.zajil.engine.Problem
import io.github.motalim_ai.zajil.engine.Tuning
import io.github.motalim_ai.zajil.video.Muxer
import io.github.motalim_ai.zajil.video.Picked
import io.github.motalim_ai.zajil.video.VideoInfo
import io.github.motalim_ai.zajil.video.VideoPages
import io.github.motalim_ai.zajil.video.YtDlp
import java.io.File
import java.io.FileNotFoundException
import java.io.IOException
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
import kotlinx.coroutines.withContext

/**
 * The app's downloads: the list the screen and the notification show, the
 * queue that decides which of them run (A2), the engine that runs them, and
 * the file that keeps the list between runs. One for the whole process
 * ([io.github.motalim_ai.zajil.ZajilApp]).
 *
 * A download is one engine download (a file), or several: a video's picture
 * and sound, joined with MediaMuxer once both are complete (A3). The engine
 * knows each by `id × STREAMS + stream`.
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
    val prefs = AppPrefs(context)

    /** Each running download's speed, for the screen. */
    val speeds = Speedometer()
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

    /** A video stream failed: why, until the download's other streams have stopped too. */
    private val failures = ConcurrentHashMap<Long, Pair<Problem, String>>()

    /** A video whose links yt-dlp is asked for again once its streams have stopped. */
    private val relink = ConcurrentHashMap.newKeySet<Long>()

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
        // A download complete before the app stopped, not yet published or joined.
        _list.value.filter { it.status == Status.Publishing }.forEach(::publish)
        _list.value.filter { it.status == Status.Merging }.forEach(::merge)
        // The network or the rules changed: stop what may not run, start
        // what may. A network that goes for a moment (a hand-over between
        // Wi-Fi and mobile data) is given a second and a half first.
        scope.launch {
            combine(network.state, settings.rules, ::Pair).collectLatest { (state, rules) ->
                if (!state.allows(rules)) {
                    delay(1_500)
                    _list.value.filter { it.status == Status.Running }.forEach {
                        requeued += it.id
                        stopEngine(it)
                    }
                }
                pump()
            }
        }
        scope.launch {
            prefs.prefs.collect { engine.setSpeedLimit(it.speedLimit?.toULong()) }
        }
    }

    /**
     * Adds [url] to the queue, unless it is on the local network ([local])
     * and Zajil may not go there yet: then it waits for the permission.
     * [name] is the user's, instead of the server's; [later] keeps it
     * paused until the user starts it.
     */
    fun add(url: String, local: Boolean, name: String? = null, later: Boolean = false): Long {
        val blocked = local && !LocalNetwork.isGranted(context)
        return insert { id, order ->
            Download(
                id = id,
                url = url,
                status = when {
                    blocked -> Status.NeedsLocalNetwork
                    later -> Status.Paused
                    else -> Status.Queued
                },
                local = local,
                order = order,
                chosenName = name,
            )
        }
    }

    /**
     * What [url] leads to (its name, size, and whether it resumes), for the
     * sheet that asks the user; only the first response's headers are read.
     */
    suspend fun probe(url: String): Result<LinkInfo> = withContext(Dispatchers.IO) {
        try {
            Result.success(engine.probe(url))
        } catch (e: EngineException) {
            Result.failure(e)
        }
    }

    /**
     * Adds a video that yt-dlp read, as the user chose it: its picture and
     * its sound, each downloaded by the engine as on the desktop.
     */
    fun addVideo(pageUrl: String, info: VideoInfo, picked: Picked, choice: String): Long {
        val streams = listOfNotNull(
            picked.video?.let { Stream(it.id, it.url, "video.${it.ext}", it.userAgent, video = true, total = it.size) },
            picked.audio.let { Stream(it.id, it.url, "audio.${it.ext}", it.userAgent, video = false, total = it.size) },
        )
        return insert { id, order ->
            Download(
                id = id,
                url = pageUrl,
                status = Status.Queued,
                order = order,
                total = picked.size,
                video = VideoJob(info.title, picked.container, streams, choice),
            )
        }
    }

    private fun insert(make: (id: Long, order: Long) -> Download): Long {
        var id = 0L
        change { list ->
            id = (list.maxOfOrNull { it.id } ?: 0) + 1
            list + make(id, (list.maxOfOrNull { it.order } ?: 0) + 1)
        }
        pump()
        return id
    }

    fun pause(id: Long) {
        heldForNetwork -= id
        val download = find(id) ?: return
        if (download.status == Status.Queued || (download.status == Status.Running && !isRunning(download))) {
            // Not with the engine (queued, or a video asking for new links).
            edit(id) { it.copy(status = Status.Paused) }
            pump()
        } else {
            stopEngine(download)
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

    /**
     * «Run the queue»: what was paused or stopped with the app goes back
     * into it, and a scheduled queue opens now instead of at its time.
     */
    fun runQueue() {
        settings.update { if (it.scheduled && !it.open) it.copy(open = true) else it }
        _list.value.filter { it.status == Status.Paused || it.status == Status.Interrupted }.forEach { enqueue(it.id) }
        pump()
    }

    /** «Stop the queue»: nothing runs or waits until the user starts it again. */
    fun stopQueue() {
        settings.update { if (it.scheduled && it.open) it.copy(open = false) else it }
        pauseAll()
    }

    /** Takes the complete downloads off the list; their files stay. */
    fun removeFinished() {
        change { list -> list.filterNot { it.status == Status.Done } }
    }

    /** Android's time for data transfers is up: everything stops, queue included. */
    fun pauseAll() {
        change { list -> list.map { if (it.status == Status.Queued) it.copy(status = Status.Paused) else it } }
        _list.value.filter { it.status == Status.Running }.forEach(::stopEngine)
    }

    /**
     * The service could not come to the foreground (after a restart of the
     * phone, from Android 15): what started stops as interrupted, and
     * resumes once the user opens Zajil.
     */
    fun stopWithoutService() {
        _list.value.filter { it.status == Status.Running }.forEach {
            interrupted += it.id
            stopEngine(it)
        }
    }

    /** The schedule's time has come: the queue runs until it is empty. */
    fun openSchedule() {
        settings.update { if (it.scheduled) it.copy(open = true) else it }
    }

    /** Stops it if it runs, then removes it with its files. */
    fun cancel(id: Long) {
        val download = find(id) ?: return
        if (isRunning(download)) {
            cancelled += id
            stopEngine(download)
        } else {
            remove(id)
        }
    }

    /** Removes it from the list; an unfinished download's files go too. */
    fun remove(id: Long) {
        val download = find(id) ?: return
        if (isRunning(download)) return
        if (download.status != Status.Done) {
            // Our own folder: deleting removes a link itself, never its target.
            download.file?.let { File("$it.zajil-part").delete() }
            download.stateFile?.let { File(it).delete() }
            download.video?.streams?.forEach { stream ->
                stream.file?.let { File(it).delete(); File("$it.zajil-part").delete() }
                stream.stateFile?.let { File(it).delete() }
            }
            download.file?.let { if (download.video != null) File(it).delete() }
        }
        // Only if empty: never anything else that might be there.
        folderOf(id).delete()
        heldForNetwork -= id
        change { list -> list.filterNot { it.id == id } }
        pump()
    }

    fun hasInterrupted() = _list.value.any { it.status == Status.Interrupted }

    /** Whether the service stays in the foreground for [list]. */
    fun keepsService(list: List<Download>): Boolean = list.any {
        it.status in working || (it.status == Status.Queued && it.id in heldForNetwork)
    }

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
        Notifications.hideStopped(context)
        ids.mapNotNull(::find).forEach { download ->
            val video = download.video
            when {
                video == null -> {
                    val stateFile = download.stateFile
                    val tuning = Tuning(
                        name = download.chosenName,
                        userAgent = null,
                        connections = prefs.prefs.value.connections.toUInt(),
                        requestSize = null,
                    )
                    if (stateFile != null && File(stateFile).exists()) {
                        engine.resumeTuned(engineId(download.id, 0), stateFile, tuning)
                    } else {
                        // No data had been written: start again from the link.
                        engine.startTuned(engineId(download.id, 0), download.url, folderOf(download.id).path, tuning)
                    }
                }
                // Links that expire within minutes: new ones first, as the
                // desktop does, then the streams go on from their bytes.
                video.streams.any { !it.finished && VideoPages.expiresSoon(it.url) } -> relinkAndStart(download.id)
                else -> startStreams(download)
            }
        }
        return true
    }

    private fun startStreams(download: Download) {
        val video = download.video ?: return
        video.streams.forEachIndexed { i, stream ->
            if (stream.finished) return@forEachIndexed
            val tuning = Tuning(
                name = stream.name,
                userAgent = stream.userAgent,
                // As the desktop gives a YouTube video's picture and sound.
                connections = if (stream.video) 16u else 8u,
                requestSize = 10uL shl 20,
            )
            val stateFile = stream.stateFile
            if (stateFile != null && File(stateFile).exists()) {
                engine.resumeTuned(engineId(download.id, i), stateFile, tuning)
            } else {
                engine.startTuned(engineId(download.id, i), stream.url, folderOf(download.id).path, tuning)
            }
        }
        // All of them were complete already (stopped while joining).
        if (video.streams.all { it.finished }) merge(download)
    }

    /**
     * Asks yt-dlp for the video's links again (they expire after some hours,
     * and YouTube refuses an old one), gives each stopped stream its new link
     * with what it has on disk kept, and starts them again.
     */
    private fun relinkAndStart(id: Long) {
        scope.launch {
            val download = find(id) ?: return@launch
            val video = download.video ?: return@launch
            val info = YtDlp.read(context, download.url).getOrElse { e ->
                fail(id, Problem.SERVER, e.message.orEmpty())
                return@launch
            }
            val streams = video.streams.map { stream ->
                val format = info.formats.firstOrNull { it.id == stream.formatId }
                    ?: return@launch fail(id, Problem.SERVER, "the format ${stream.formatId} is gone from the page")
                val stateFile = stream.stateFile
                var kept = stream
                if (!stream.finished && stateFile != null && File(stateFile).exists()) {
                    try {
                        engine.refreshLink(stateFile, format.url)
                    } catch (_: Exception) {
                        // Not the same file after all: that stream starts over.
                        stream.file?.let { File("$it.zajil-part").delete() }
                        File(stateFile).delete()
                        kept = stream.copy(stateFile = null, done = 0)
                    }
                }
                kept.copy(url = format.url, userAgent = format.userAgent)
            }
            edit(id) { it.copy(video = video.copy(streams = streams, refreshed = true)) }
            // Paused or removed while yt-dlp was asked: nothing starts.
            find(id)?.takeIf { it.status == Status.Running }?.let(::startStreams)
        }
    }

    private fun fail(id: Long, problem: Problem, detail: String) {
        edit(id) { it.copy(status = Status.Failed, problem = problem, detail = detail.take(300)) }
        pump()
    }

    /** Joins a video's complete streams into one file, then publishes it. */
    private fun merge(download: Download) {
        val video = download.video ?: return
        edit(download.id) { it.copy(status = Status.Merging) }
        scope.launch {
            val inputs = video.streams.mapNotNull { it.file?.let(::File) }
            val folder = folderOf(download.id)
            val output = File(folder, "${safeName(video.title, download.id)}.${video.container.ext}")
            if (!Muxer.hasRoom(inputs, folder)) {
                fail(download.id, Problem.NOT_ENOUGH_SPACE, "no room to join the picture and the sound")
                return@launch
            }
            try {
                Muxer.merge(inputs, output, video.container)
            } catch (e: IOException) {
                fail(download.id, Problem.OTHER, e.message.orEmpty())
                return@launch
            }
            inputs.forEach(File::delete)
            edit(download.id) { it.copy(status = Status.Publishing, file = output.path) }
            find(download.id)?.let(::publish)
        }
    }

    /** Queued again; false if it cannot be (running, done, or no permission). */
    private fun enqueue(id: Long): Boolean {
        val download = find(id) ?: return false
        if (download.status !in resumable || isRunning(download)) return false
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
                    it.copy(
                        status = Status.Done,
                        uri = published.uri.toString(),
                        shownName = published.name,
                        finishedAt = System.currentTimeMillis(),
                    )
                } else {
                    it.copy(status = Status.Failed, problem = Problem.OTHER, stateFile = null)
                }
            }
        }
    }

    private fun streamsOf(download: Download) = download.video?.streams?.size ?: 1

    private fun isRunning(download: Download) =
        (0 until streamsOf(download)).any { engine.isRunning(engineId(download.id, it)) }

    private fun stopEngine(download: Download) {
        (0 until streamsOf(download)).forEach { engine.pause(engineId(download.id, it)) }
    }

    /** Where a download goes once the engine has stopped it. */
    private fun stoppedAs(id: Long, otherwise: Status): Status = when {
        interrupted.remove(id) -> Status.Interrupted
        requeued.remove(id) -> Status.Queued.also { heldForNetwork += id }
        else -> otherwise.also { if (it == Status.Queued) heldForNetwork += id }
    }

    private fun editStream(id: Long, stream: Int, how: (Stream) -> Stream) {
        edit(id) { download ->
            val video = download.video ?: return@edit download
            val streams = video.streams.mapIndexed { i, s -> if (i == stream) how(s) else s }
            download.copy(video = video.copy(streams = streams)).withTotals()
        }
    }

    private inner class EngineListener : Listener {
        override fun started(id: Long, file: String, stateFile: String, total: ULong?, from: ULong) {
            val (owner, stream) = split(id)
            if (find(owner)?.video != null) {
                editStream(owner, stream) {
                    it.copy(file = file, stateFile = stateFile, total = total?.toLong() ?: it.total, done = from.toLong())
                }
            } else {
                edit(owner) {
                    it.copy(file = file, stateFile = stateFile, total = total?.toLong(), done = from.toLong())
                }
            }
        }

        override fun progress(id: Long, done: ULong, total: ULong?) {
            val (owner, stream) = split(id)
            // Not saved: the state files are what resuming trusts.
            _list.update { list ->
                list.map { d ->
                    when {
                        d.id != owner -> d
                        d.video == null -> d.copy(done = done.toLong(), total = total?.toLong())
                        else -> {
                            val streams = d.video.streams.mapIndexed { i, s ->
                                if (i == stream) s.copy(done = done.toLong(), total = total?.toLong() ?: s.total) else s
                            }
                            d.copy(video = d.video.copy(streams = streams)).withTotals()
                        }
                    }
                }
            }
            find(owner)?.let { speeds.report(owner, it.done) }
        }

        override fun finished(id: Long, file: String, bytes: ULong) {
            val (owner, stream) = split(id)
            val download = find(owner) ?: return
            if (download.video == null) {
                requeued.remove(owner)
                interrupted.remove(owner)
                edit(owner) {
                    it.copy(status = Status.Publishing, file = file, done = bytes.toLong(), total = bytes.toLong())
                }
                find(owner)?.let(::publish)
            } else {
                editStream(owner, stream) {
                    it.copy(file = file, finished = true, done = bytes.toLong(), total = bytes.toLong())
                }
                val now = find(owner) ?: return
                if (now.video!!.streams.all { it.finished }) {
                    requeued.remove(owner)
                    interrupted.remove(owner)
                    merge(now)
                } else if (!isRunning(now)) {
                    // The other stream had stopped: this one finishing ends the run.
                    ended(owner)
                }
            }
            pump()
        }

        override fun paused(id: Long, stateFile: String?) {
            val (owner, stream) = split(id)
            val download = find(owner) ?: return
            if (download.video == null) {
                val status = stoppedAs(owner, Status.Paused)
                edit(owner) { it.copy(status = status, stateFile = stateFile ?: it.stateFile) }
                if (cancelled.remove(owner)) remove(owner) else pump()
                return
            }
            editStream(owner, stream) { it.copy(stateFile = stateFile ?: it.stateFile) }
            if (!isRunning(download)) ended(owner)
        }

        override fun failed(id: Long, problem: Problem, detail: String, stateFile: String?) {
            val (owner, stream) = split(id)
            val download = find(owner) ?: return
            // The connection failed because the network went: it waits in
            // the queue for the network, instead of failing.
            val offline = problem == Problem.NETWORK && !network.state.value.allows(settings.rules.value)
            if (download.video == null) {
                val status = stoppedAs(owner, if (offline) Status.Queued else Status.Failed)
                edit(owner) {
                    it.copy(
                        status = status,
                        problem = problem.takeIf { status == Status.Failed },
                        detail = detail.takeIf { status == Status.Failed },
                        stateFile = stateFile ?: it.stateFile,
                    )
                }
                if (cancelled.remove(owner)) remove(owner) else pump()
                return
            }
            editStream(owner, stream) { it.copy(stateFile = stateFile ?: it.stateFile) }
            when {
                offline -> requeued += owner
                // YouTube refused an old link: yt-dlp gives new ones, once.
                problem == Problem.SERVER && !download.video.refreshed -> relink += owner
                else -> failures[owner] = problem to detail
            }
            // The other stream stops too; the download ends when both have.
            stopEngine(download)
            if (!isRunning(download)) ended(owner)
        }
    }

    /** All of a video's streams have stopped: what becomes of it. */
    private fun ended(id: Long) {
        if (cancelled.remove(id)) {
            failures.remove(id)
            relink.remove(id)
            remove(id)
            return
        }
        if (relink.remove(id) && failures[id] == null && id !in requeued && id !in interrupted) {
            edit(id) { it.copy(status = Status.Running) }
            relinkAndStart(id)
            return
        }
        val failure = failures.remove(id)
        val status = stoppedAs(id, if (failure != null) Status.Failed else Status.Paused)
        edit(id) {
            it.copy(
                status = status,
                problem = failure?.first.takeIf { status == Status.Failed },
                detail = failure?.second.takeIf { status == Status.Failed },
            )
        }
        pump()
    }

    private fun edit(id: Long, how: (Download) -> Download) {
        change { list -> list.map { if (it.id == id) how(it) else it } }
    }

    private fun change(how: (List<Download>) -> List<Download>) {
        _list.update(how)
        speeds.keepOnly(_list.value.filter { it.status == Status.Running }.map { it.id }.toSet())
        scope.launch(writer) { save(_list.value) }
    }

    /** A failed save (a full disk) keeps the last good file; the next change tries again. */
    private fun save(list: List<Download>) {
        val out = try {
            listFile.startWrite()
        } catch (_: IOException) {
            return
        }
        try {
            out.write(Download.listToJson(list).toByteArray())
            listFile.finishWrite(out)
        } catch (_: IOException) {
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
        /** Engine downloads per download: a video's picture and sound, and room to spare. */
        const val STREAMS = 4L

        fun engineId(id: Long, stream: Int) = id * STREAMS + stream

        fun split(engineId: Long) = (engineId / STREAMS) to (engineId % STREAMS).toInt()

        /** A file name from a video's title: nothing a folder would refuse. */
        fun safeName(title: String, id: Long): String {
            val name = title.map { c -> if (c.isISOControl() || c in "/\\:*?\"<>|") '_' else c }
                .joinToString("").trim().trimEnd('.').take(120)
            return name.ifBlank { "video-$id" }
        }

        val resumable = setOf(Status.Interrupted, Status.Paused, Status.Failed, Status.NeedsLocalNetwork)
        val active = setOf(Status.Queued, Status.Running)

        /** The service stays for these: the engine works, or the file is being made. */
        val working = setOf(Status.Running, Status.Merging, Status.Publishing)
    }
}

/** A video's progress is its streams' together. */
private fun Download.withTotals(): Download {
    val streams = video?.streams ?: return this
    val totals = streams.map { it.total }
    return copy(
        done = streams.sumOf { it.done },
        total = if (totals.all { it != null }) totals.sumOf { it!! } else total,
    )
}
