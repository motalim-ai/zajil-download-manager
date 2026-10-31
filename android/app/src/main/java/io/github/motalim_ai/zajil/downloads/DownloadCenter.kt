package io.github.motalim_ai.zajil.downloads

import android.content.Context
import android.os.Environment
import android.util.AtomicFile
import io.github.motalim_ai.zajil.engine.Downloads
import io.github.motalim_ai.zajil.engine.Listener
import io.github.motalim_ai.zajil.engine.Problem
import java.io.File
import java.io.FileNotFoundException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * The app's downloads: the list the screen and the notification show, the
 * engine that runs them, and the file that keeps the list between runs.
 * One for the whole process ([io.github.motalim_ai.zajil.ZajilApp]).
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
    private val _list = MutableStateFlow(load())
    val list: StateFlow<List<Download>> = _list.asStateFlow()
    private val engine = Downloads(EngineListener())

    /** Cancelled while running: removed once the engine has stopped. */
    private val cancelled = java.util.concurrent.ConcurrentHashMap.newKeySet<Long>()

    /**
     * Where unfinished downloads live: the app's own folder on the shared
     * storage, which needs no permission (ADR-0012, section 4).
     */
    private val workDir: File
        get() = context.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS)
            ?: File(context.filesDir, "downloads")

    init {
        // A download complete before the app stopped, not yet published.
        _list.value.filter { it.status == Status.Publishing }.forEach(::publish)
    }

    fun add(url: String): Long {
        var id = 0L
        change { list ->
            id = (list.maxOfOrNull { it.id } ?: 0) + 1
            list + Download(id = id, url = url, status = Status.Running)
        }
        engine.start(id, url, workDir.path)
        return id
    }

    fun pause(id: Long) {
        engine.pause(id)
    }

    fun resume(id: Long) {
        val download = find(id) ?: return
        if (download.status !in resumable || engine.isRunning(id)) return
        edit(id) { it.copy(status = Status.Running, problem = null, detail = null) }
        val stateFile = download.stateFile
        if (stateFile != null && File(stateFile).exists()) {
            engine.resume(id, stateFile)
        } else {
            // No data had been written: start again from the link.
            engine.start(id, download.url, workDir.path)
        }
    }

    /** Resumes what was running when the app or the phone stopped. */
    fun resumeInterrupted() {
        _list.value.filter { it.status == Status.Interrupted }.forEach { resume(it.id) }
    }

    fun resumeAll() {
        _list.value.filter { it.status in resumable }.forEach { resume(it.id) }
    }

    fun pauseAll() {
        _list.value.filter { it.status == Status.Running }.forEach { pause(it.id) }
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
        change { list -> list.filterNot { it.id == id } }
    }

    fun hasRunning() = _list.value.any { it.status == Status.Running }

    fun hasInterrupted() = _list.value.any { it.status == Status.Interrupted }

    fun find(id: Long) = _list.value.firstOrNull { it.id == id }

    private fun publish(download: Download) {
        scope.launch {
            val file = download.file?.let(::File)
            val published = file?.let { Publisher.publish(context, it) }
            edit(download.id) {
                if (published != null) {
                    it.copy(status = Status.Done, uri = published.uri.toString(), shownName = published.name)
                } else {
                    it.copy(status = Status.Failed, problem = Problem.OTHER, stateFile = null)
                }
            }
        }
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
            edit(id) {
                it.copy(status = Status.Publishing, file = file, done = bytes.toLong(), total = bytes.toLong())
            }
            find(id)?.let(::publish)
        }

        override fun paused(id: Long, stateFile: String?) {
            edit(id) { it.copy(status = Status.Paused, stateFile = stateFile ?: it.stateFile) }
            if (cancelled.remove(id)) remove(id)
        }

        override fun failed(id: Long, problem: Problem, detail: String, stateFile: String?) {
            edit(id) {
                it.copy(
                    status = Status.Failed,
                    problem = problem,
                    detail = detail,
                    stateFile = stateFile ?: it.stateFile,
                )
            }
            if (cancelled.remove(id)) remove(id)
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
        val resumable = setOf(Status.Interrupted, Status.Paused, Status.Failed)
    }
}
