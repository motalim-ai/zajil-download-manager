package io.github.motalim_ai.zajil.downloads

import android.content.ContentValues
import android.content.Context
import android.media.MediaScannerConnection
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.webkit.MimeTypeMap
import io.github.motalim_ai.zajil.R
import java.io.File
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine

/**
 * Moves a finished download from the app's own folder to where the user
 * keeps them (ADR-0012, section 4): the public Download/Zajil folder, where
 * the Files app and the gallery see it, or a folder the user chose with
 * Android's folder picker (0.3.0); with «A folder for each type», into its
 * category's folder there.
 */
object Publisher {
    private const val FOLDER = "Zajil"

    /** Where a finished file went: its content URI and the name it got there. */
    data class Published(val uri: Uri, val name: String)

    /**
     * Moves [file]; null if it could not be moved. The name may differ: the
     * folder already has one, so it gets "name (1).ext". A chosen folder
     * that can no longer be written (its permission gone) gives way to
     * Download/Zajil: the file is never lost.
     */
    suspend fun publish(context: Context, file: File, prefs: Prefs): Published? {
        if (!file.isFile) return null
        val sub = if (prefs.folderPerType) SaveCategory.of(file.name)?.let { folderName(context, it) } else null
        prefs.saveTree?.let { tree -> runCatching { throughTree(context, file, Uri.parse(tree), sub) }.getOrNull()?.let { return it } }
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            throughMediaStore(context, file, sub)
        } else {
            throughPath(context, file, sub)
        }
    }

    /** A category's folder, named in the app's language as on the desktop. */
    fun folderName(context: Context, category: SaveCategory): String = context.getString(
        when (category) {
            SaveCategory.Programs -> R.string.folder_programs
            SaveCategory.Video -> R.string.folder_video
            SaveCategory.Music -> R.string.folder_music
            SaveCategory.Documents -> R.string.folder_documents
            SaveCategory.Archives -> R.string.folder_archives
        },
    )

    /** The chosen folder as a path to show («Movies/Zajil»), from its tree URI. */
    fun shownTree(tree: String): String {
        val id = runCatching { DocumentsContract.getTreeDocumentId(Uri.parse(tree)) }.getOrNull() ?: return tree
        // «primary:Movies/Zajil»: the phone's own storage, then the path; another volume keeps its name.
        val (volume, path) = id.substringBefore(':') to id.substringAfter(':', "")
        return if (volume == "primary") path.ifEmpty { "/" } else "$volume/$path".trimEnd('/')
    }

    private fun mimeType(file: File): String =
        MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase())
            ?: "application/octet-stream"

    /** The folder the user chose: Android's Storage Access Framework, with the permission it kept. */
    private fun throughTree(context: Context, file: File, tree: Uri, sub: String?): Published? {
        val resolver = context.contentResolver
        var dir = DocumentsContract.getTreeDocumentId(tree)
        if (sub != null) dir = childFolder(context, tree, dir, sub) ?: return null
        val dirUri = DocumentsContract.buildDocumentUriUsingTree(tree, dir)
        val uri = DocumentsContract.createDocument(resolver, dirUri, mimeType(file), file.name) ?: return null
        return try {
            resolver.openOutputStream(uri)!!.use { out -> file.inputStream().use { it.copyTo(out) } }
            file.delete()
            val name = resolver.query(uri, arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME), null, null, null)
                ?.use { if (it.moveToFirst()) it.getString(0) else null }
            Published(uri, name ?: file.name)
        } catch (e: Exception) {
            runCatching { DocumentsContract.deleteDocument(resolver, uri) }
            null
        }
    }

    /** [name]'s folder in [parent] of [tree], made if it is not there. */
    private fun childFolder(context: Context, tree: Uri, parent: String, name: String): String? {
        val resolver = context.contentResolver
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, parent)
        val columns = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
        )
        resolver.query(children, columns, null, null, null)?.use { c ->
            while (c.moveToNext()) {
                if (c.getString(1) == name && c.getString(2) == DocumentsContract.Document.MIME_TYPE_DIR) return c.getString(0)
            }
        }
        val parentUri = DocumentsContract.buildDocumentUriUsingTree(tree, parent)
        return DocumentsContract.createDocument(resolver, parentUri, DocumentsContract.Document.MIME_TYPE_DIR, name)
            ?.let(DocumentsContract::getDocumentId)
    }

    /** Android 10 and later: MediaStore, no permission. */
    private fun throughMediaStore(context: Context, file: File, sub: String?): Published? {
        val resolver = context.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, file.name)
            put(MediaStore.Downloads.MIME_TYPE, mimeType(file))
            put(MediaStore.Downloads.RELATIVE_PATH, listOfNotNull(Environment.DIRECTORY_DOWNLOADS, FOLDER, sub).joinToString("/"))
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
    private suspend fun throughPath(context: Context, file: File, sub: String?): Published? {
        val zajil = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), FOLDER)
        val folder = if (sub != null) File(zajil, sub) else zajil
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
