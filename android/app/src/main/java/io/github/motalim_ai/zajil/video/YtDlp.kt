package io.github.motalim_ai.zajil.video

import android.content.Context
import android.os.SystemClock
import android.util.Log
import com.chaquo.python.Python
import com.chaquo.python.android.AndroidPlatform
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/**
 * yt-dlp inside the app (ADR-0012, section 6), through Chaquopy's Python:
 * it reads a video page and says which formats there are. One call at a
 * time; Python starts at the first.
 */
object YtDlp {
    private const val TAG = "ZajilVideo"
    private val lock = Mutex()

    /** The video at [url] and its formats; never logs the link. */
    suspend fun read(context: Context, url: String): Result<VideoInfo> = withContext(Dispatchers.IO) {
        lock.withLock {
            val started = SystemClock.elapsedRealtime()
            val python = python(context)
            val ready = SystemClock.elapsedRealtime()
            val json = python.getModule("zajil_video")
                .callAttr("formats", url, quickJs(context), ToolsUpdate.ytDlpZip(context)?.path, cacheDir(context))
                .toString()
            Log.i(TAG, "Python ready in ${ready - started} ms, the page read in ${SystemClock.elapsedRealtime() - ready} ms")
            VideoInfo.parse(json)
        }
    }

    /** The version of the yt-dlp in use. */
    suspend fun version(context: Context): String = withContext(Dispatchers.IO) {
        lock.withLock {
            python(context).getModule("zajil_video").callAttr("version", ToolsUpdate.ytDlpZip(context)?.path).toString()
        }
    }

    private fun python(context: Context): Python {
        if (!Python.isStarted()) Python.start(AndroidPlatform(context.applicationContext))
        return Python.getInstance()
    }

    /** yt-dlp's cache, in the app's own cache folder (Android may empty it). */
    private fun cacheDir(context: Context): String = File(context.cacheDir, "yt-dlp").path

    /**
     * QuickJS-NG, for YouTube's JavaScript challenges: shipped in the APK
     * as a native library, so that Android unpacks it where a program may
     * run (it never runs from the app's data folder).
     */
    private fun quickJs(context: Context): String =
        File(context.applicationInfo.nativeLibraryDir, "libqjs.so").path
}
