package io.github.motalim_ai.zajil.downloads

import android.content.ContentValues
import android.content.Context
import android.media.MediaScannerConnection
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.webkit.MimeTypeMap
import java.io.File
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine

/**
 * Moves a finished download from the app's own folder to the public
 * Download/Zajil folder, where the Files app and the gallery see it
 * (ADR-0012, section 4).
 */
object Publisher {
    private const val FOLDER = "Zajil"

    /** Where a finished file went: its content URI and the name it got there. */
    data class Published(val uri: Uri, val name: String)

    /**
     * Moves [file]; null if it could not be moved. The name may differ: the
     * folder already has one, so it gets "name (1).ext".
     */
    suspend fun publish(context: Context, file: File): Published? {
        if (!file.isFile) return null
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            throughMediaStore(context, file)
        } else {
            throughPath(context, file)
        }
    }

    private fun mimeType(file: File): String =
        MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase())
            ?: "application/octet-stream"

    /** Android 10 and later: MediaStore, no permission. */
    private fun throughMediaStore(context: Context, file: File): Published? {
        val resolver = context.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, file.name)
            put(MediaStore.Downloads.MIME_TYPE, mimeType(file))
            put(MediaStore.Downloads.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/$FOLDER")
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: return null
        return try {
            resolver.openOutputStream(uri)!!.use { out -> file.inputStream().use { it.copyTo(out) } }
            resolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
            file.delete()
            val name = resolver.query(uri, arrayOf(MediaStore.Downloads.DISPLAY_NAME), null, null, null)
                ?.use { if (it.moveToFirst()) it.getString(0) else null }
            Published(uri, name ?: file.name)
        } catch (e: Exception) {
            resolver.delete(uri, null, null)
            null
        }
    }

    /**
     * Android 8 and 9: the folder by its path, with WRITE_EXTERNAL_STORAGE
     * (asked before the first download). A free name like the engine's own:
     * "name (1).ext".
     */
    @Suppress("DEPRECATION")
    private suspend fun throughPath(context: Context, file: File): Published? {
        val folder = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), FOLDER)
        if (!folder.isDirectory && !folder.mkdirs()) return null
        val stem = file.nameWithoutExtension
        val dot = if (file.extension.isEmpty()) "" else ".${file.extension}"
        val target = generateSequence(0) { it + 1 }
            .map { n -> File(folder, if (n == 0) file.name else "$stem ($n)$dot") }
            .first { !it.exists() }
        if (!file.renameTo(target)) {
            try {
                file.copyTo(target)
            } catch (_: Exception) {
                target.delete()
                return null
            }
            file.delete()
        }
        val uri = suspendCancellableCoroutine { done ->
            MediaScannerConnection.scanFile(context, arrayOf(target.path), arrayOf(mimeType(target))) { _, uri ->
                done.resume(uri ?: Uri.fromFile(target))
            }
        }
        return Published(uri, target.name)
    }
}
