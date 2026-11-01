package io.github.motalim_ai.zajil.video

import android.app.Service
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.Binder
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.Parcel
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.util.Log
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

/** What QuickJS-NG gave back: its exit code, what it printed, and its errors. */
class JsRun(@JvmField val code: Int, @JvmField val stdout: String, @JvmField val stderr: String)

/**
 * YouTube's player code, for its JavaScript challenges, runs here only
 * (ADR-0009's rule on Android, the owner's decision of 2026-11-01): in
 * QuickJS-NG started by [JsBoxService], an isolated process with a user of
 * its own, no permissions, no network and none of the app's files; with
 * QuickJS's memory and stack limits and a time limit. Never in the app's
 * process. A new QuickJS process for each run; the isolated process stays
 * [KEEP_MS] after the last one, then ends.
 */
object JsBox {
    private const val TAG = "ZajilVideo"

    /** The longest a challenge may take, as the desktop's `zajil-jsbox`. */
    const val TIME_LIMIT_SECONDS = 60L

    /** The most bytes of a script or of its output, as the desktop's. */
    const val MAX_BYTES = 32 shl 20

    /** For yt-dlp (zajil_video.py): `run(script)` from Python's thread. */
    class Runner(private val context: Context) {
        fun run(script: String): JsRun = JsBox.run(context, script)
    }

    /** Runs [script] in the isolated process. Blocks; never on the main thread. */
    fun run(context: Context, script: String): JsRun {
        val started = SystemClock.elapsedRealtime()
        return runIsolated(context, script).also {
            // How long, and how big: never the script itself.
            Log.i(TAG, "QuickJS in the isolated process: ${script.length / 1024} KiB in ${SystemClock.elapsedRealtime() - started} ms, exit ${it.code}")
        }
    }

    /** How long the isolated process stays after its last run, ready for the next one. */
    private const val KEEP_MS = 30_000L

    private val main = Handler(Looper.getMainLooper())
    private val lock = Any()
    private var bound: Bound? = null

    /** The isolated process while it is bound, and its connection. */
    private class Bound(val connection: ServiceConnection, val binder: IBinder)

    private val unbind = Runnable {
        synchronized(lock) {
            // Unbound, the isolated process ends: nothing of the last runs stays.
            bound?.let { runCatching { appContext?.unbindService(it.connection) } }
            bound = null
        }
    }

    @Volatile private var appContext: Context? = null

    /**
     * The isolated process, started if it is not there. It stays a little
     * after a run (each run still has a QuickJS process of its own), so
     * that a page's several challenges do not each start it again.
     */
    private fun service(app: Context): IBinder? {
        synchronized(lock) {
            main.removeCallbacks(unbind)
            bound?.takeIf { it.binder.isBinderAlive }?.let { return it.binder }
            bound?.let { runCatching { app.unbindService(it.connection) } }
            bound = null
        }
        appContext = app
        val connected = CountDownLatch(1)
        var binder: IBinder? = null
        val connection = object : ServiceConnection {
            override fun onServiceConnected(name: ComponentName, service: IBinder) {
                binder = service
                connected.countDown()
            }

            override fun onServiceDisconnected(name: ComponentName) {
                synchronized(lock) { if (bound?.connection === this) bound = null }
            }

            override fun onBindingDied(name: ComponentName) = connected.countDown()

            override fun onNullBinding(name: ComponentName) = connected.countDown()
        }
        if (!app.bindService(Intent(app, JsBoxService::class.java), connection, Context.BIND_AUTO_CREATE)) return null
        val service = if (connected.await(10, TimeUnit.SECONDS)) binder else null
        if (service == null) {
            runCatching { app.unbindService(connection) }
            return null
        }
        synchronized(lock) { bound = Bound(connection, service) }
        return service
    }

    private fun runIsolated(context: Context, script: String): JsRun {
        val app = context.applicationContext
        try {
            val service = service(app) ?: return JsRun(-1, "", "the isolated process did not start")
            return call(service, script.toByteArray(Charsets.UTF_8))
        } finally {
            main.postDelayed(unbind, KEEP_MS)
        }
    }

    private fun call(service: IBinder, script: ByteArray): JsRun {
        if (script.size > MAX_BYTES) return JsRun(-1, "", "the script is too large")
        val (scriptRead, scriptWrite) = ParcelFileDescriptor.createPipe()
        val (outRead, outWrite) = ParcelFileDescriptor.createPipe()
        val writer = thread(name = "jsbox-in") {
            runCatching { ParcelFileDescriptor.AutoCloseOutputStream(scriptWrite).use { it.write(script) } }
        }
        var output = ByteArray(0)
        val reader = thread(name = "jsbox-out") {
            output = runCatching { ParcelFileDescriptor.AutoCloseInputStream(outRead).use { readCapped(it) } }.getOrDefault(ByteArray(0))
        }
        val data = Parcel.obtain()
        val reply = Parcel.obtain()
        var sent = false
        try {
            // The parcel keeps copies of both ends it carries until it is recycled: it is,
            // right after the call, so that the output pipe ends there (or nothing would
            // ever end the reader's wait).
            data.writeFileDescriptor(scriptRead.fileDescriptor)
            data.writeFileDescriptor(outWrite.fileDescriptor)
            val accepted = service.transact(JsBoxService.RUN, data, reply, 0)
            data.recycle()
            sent = true
            scriptRead.close()
            outWrite.close()
            if (!accepted) return JsRun(-1, "", "the isolated process refused")
            val code = reply.readInt()
            val stderr = reply.readString().orEmpty()
            writer.join(TimeUnit.SECONDS.toMillis(5))
            reader.join(TimeUnit.SECONDS.toMillis(10))
            // Never half an answer.
            if (reader.isAlive) return JsRun(-3, "", "the isolated process's output did not end")
            return JsRun(code, String(output, Charsets.UTF_8), stderr)
        } catch (e: Exception) {
            return JsRun(-1, "", "the isolated process stopped: ${e.javaClass.simpleName}")
        } finally {
            if (!sent) data.recycle()
            reply.recycle()
            runCatching { scriptRead.close() }
            runCatching { outWrite.close() }
            // A reader still waiting ends with its pipe.
            runCatching { outRead.close() }
        }
    }

    /** At most [MAX_BYTES] of [from]; the rest is read and dropped. */
    internal fun readCapped(from: InputStream, max: Int = MAX_BYTES): ByteArray {
        val out = ByteArrayOutputStream()
        val buffer = ByteArray(64 * 1024)
        while (true) {
            val n = from.read(buffer)
            if (n < 0) break
            val room = max - out.size()
            if (room > 0) out.write(buffer, 0, minOf(n, room))
        }
        return out.toByteArray()
    }

    /** Copies at most [max] bytes of [from] to [to], then closes [to]. */
    internal fun copyCapped(from: InputStream, to: OutputStream, max: Int = MAX_BYTES) {
        to.use {
            val buffer = ByteArray(64 * 1024)
            var total = 0
            while (total < max) {
                val n = from.read(buffer, 0, minOf(buffer.size, max - total))
                if (n < 0) break
                it.write(buffer, 0, n)
                total += n
            }
        }
    }
}

/**
 * The isolated process (`android:isolatedProcess`, process `:jsbox`): runs
 * the app's QuickJS-NG on the script the app hands over, and nothing else.
 * Only the app itself may call it.
 */
class JsBoxService : Service() {
    private val binder = object : Binder() {
        override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
            if (code != RUN || reply == null) return super.onTransact(code, data, reply, flags)
            // Only the app that started this process.
            if (getCallingUid() != applicationInfo.uid) return false
            val script = data.readFileDescriptor() ?: return false
            val out = data.readFileDescriptor() ?: return false
            val (exit, stderr) = ParcelFileDescriptor.AutoCloseInputStream(script).use { input ->
                ParcelFileDescriptor.AutoCloseOutputStream(out).use { output -> runQuickJs(input, output) }
            }
            reply.writeInt(exit)
            reply.writeString(stderr)
            return true
        }
    }

    override fun onBind(intent: Intent): IBinder = binder

    /** QuickJS-NG on [script], its output to [out]: its exit code and its first errors. */
    private fun runQuickJs(script: InputStream, out: OutputStream): Pair<Int, String> {
        val qjs = File(applicationInfo.nativeLibraryDir, "libqjs.so")
        val started = SystemClock.elapsedRealtime()
        val process = ProcessBuilder(
            qjs.path,
            // QuickJS's own limits, as the desktop's zajil-jsbox: 1 GiB of memory, an 8 MiB stack.
            "--memory-limit", "1048576",
            "--stack-size", "8192",
            "--std",
            "-e", BOOTSTRAP,
        ).apply {
            environment().clear()
            directory(File("/"))
        }.start()
        val feed = thread(name = "qjs-in") { runCatching { JsBox.copyCapped(script, process.outputStream) } }
        val drain = thread(name = "qjs-out") { runCatching { JsBox.copyCapped(process.inputStream, out) } }
        var errors = ByteArray(0)
        val drainErrors = thread(name = "qjs-err") { errors = runCatching { JsBox.readCapped(process.errorStream, 64 * 1024) }.getOrDefault(ByteArray(0)) }
        val finished = process.waitFor(JsBox.TIME_LIMIT_SECONDS, TimeUnit.SECONDS)
        if (!finished) process.destroyForcibly().waitFor()
        feed.join(1000)
        drain.join(5000)
        drainErrors.join(1000)
        val code = if (finished) process.exitValue() else -2
        Log.i("ZajilVideo", "QuickJS ran ${SystemClock.elapsedRealtime() - started} ms in the isolated process")
        val message = if (finished) String(errors, Charsets.UTF_8) else "QuickJS took longer than ${JsBox.TIME_LIMIT_SECONDS} s"
        return code to message
    }

    companion object {
        const val RUN = IBinder.FIRST_CALL_TRANSACTION

        /**
         * Reads the script from stdin (QuickJS reads no script from a pipe
         * by itself), takes `std`, `os` and `bjson` away from the global
         * object, and runs it as a classic script, as yt-dlp's `--script`.
         */
        const val BOOTSTRAP =
            "(function(){var t=std.in.readAsString();delete globalThis.std;delete globalThis.os;delete globalThis.bjson;(0,eval)(t)})()"
    }
}
