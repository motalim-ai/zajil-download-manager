package io.github.motalim_ai.zajil.video

import android.content.Context
import android.os.SystemClock
import android.util.Log
import com.chaquo.python.Python
import com.chaquo.python.android.AndroidPlatform
import io.github.motalim_ai.zajil.protection
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

    /** The video at [url] and its formats; a playlist is a failure here. Never logs the link. */
    suspend fun read(
        context: Context,
        url: String,
        cookies: String? = null,
        referer: String? = null,
    ): Result<VideoInfo> = readPage(context, url, cookies, referer).mapCatching { page ->
        when (page) {
            is Page.Video -> page.info
            is Page.Playlist -> throw VideoError("a playlist, not a video")
        }
    }

    /** What [url] holds: a video and its formats, or a playlist listed (not opened). */
    suspend fun readPage(
        context: Context,
        url: String,
        cookies: String? = null,
        referer: String? = null,
    ): Result<Page> = withContext(Dispatchers.IO) {
        lock.withLock {
            val started = SystemClock.elapsedRealtime()
            val python = python(context)
            val ready = SystemClock.elapsedRealtime()
            val json = python.getModule("zajil_video")
                .callAttr(
                    "formats",
                    url,
                    quickJs(context),
                    ToolsUpdate.ytDlpZip(context)?.path,
                    cacheDir(context),
                    cookies,
                    referer,
                    JsBox.Runner(context.applicationContext),
                    context.protection.hostCheck,
                )
                .toString()
            Log.i(TAG, "Python ready in ${ready - started} ms, the page read in ${SystemClock.elapsedRealtime() - ready} ms")
            Page.parse(json)
        }
    }

    /** What [downloadHls] tells, and asks, while it works. Called on Python's thread. */
    interface HlsListener {
        fun progress(done: Long, total: Long)

        fun stopping(): Boolean
    }

    /** How an HLS download ended. */
    sealed interface HlsEnd {
        data class Done(val file: java.io.File) : HlsEnd

        data object Stopped : HlsEnd

        data class Failed(val detail: String) : HlsEnd
    }

    /**
     * Downloads HLS format [formatId] of [url] into [folder] with yt-dlp's
     * own fragment downloader (A6): the engine does not download HLS. Again
     * in the same folder, it continues from the fragments already there.
     * Not under the lock: reading a page may go on meanwhile.
     */
    suspend fun downloadHls(
        context: Context,
        url: String,
        formatId: String,
        folder: java.io.File,
        listener: HlsListener,
        cookies: String?,
        referer: String?,
        /** The file's name in [folder], without its extension ("stream", or DASH's "video" and "audio"). */
        name: String = "stream",
    ): HlsEnd = withContext(Dispatchers.IO) {
        folder.mkdirs()
        val json = runCatching {
            python(context).getModule("zajil_video")
                .callAttr("download_hls", url, formatId, folder.path, listener, ToolsUpdate.ytDlpZip(context)?.path, cookies, referer, name, context.protection.hostCheck)
                .toString()
        }.getOrElse { return@withContext HlsEnd.Failed(it.message.orEmpty()) }
        val o = org.json.JSONObject(json)
        when {
            o.optBoolean("stopped") -> HlsEnd.Stopped
            o.has("file") -> HlsEnd.Done(java.io.File(o.getString("file")))
            else -> HlsEnd.Failed(o.optString("error"))
        }
    }

    /** The version of the yt-dlp in use. */
    suspend fun version(context: Context): String = withContext(Dispatchers.IO) {
        lock.withLock {
            python(context).getModule("zajil_video").callAttr("version", ToolsUpdate.ytDlpZip(context)?.path).toString()
        }
    }

    @Synchronized
    private fun python(context: Context): Python {
        if (!Python.isStarted()) Python.start(AndroidPlatform(context.applicationContext))
        return Python.getInstance()
    }

    /** yt-dlp's cache, in the app's own cache folder (Android may empty it). */
    private fun cacheDir(context: Context): String = File(context.cacheDir, "yt-dlp").path

    /**
     * QuickJS-NG, for YouTube's JavaScript challenges: shipped in the APK
     * as a native library, so that Android unpacks it where a program may
     * run (it never runs from the app's data folder). It runs only in
     * [JsBoxService]'s isolated process; yt-dlp is told its path, never runs it.
     */
    private fun quickJs(context: Context): String =
        File(context.applicationInfo.nativeLibraryDir, "libqjs.so").path
}
