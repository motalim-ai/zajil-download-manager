package io.github.motalim_ai.zajil.video

import android.content.Context
import io.github.motalim_ai.zajil.BuildConfig
import io.github.motalim_ai.zajil.engine.AndroidTools
import io.github.motalim_ai.zajil.engine.checkAndroidTools
import io.github.motalim_ai.zajil.update.Https
import java.io.File
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * A newer yt-dlp than the one in the APK, from Android's own signed list
 * (ADR-0012, section 8): `tools-android.json` on the `android-channel`
 * pre-release, signed by one of the project's two update keys, checked in
 * Rust. Only when the user asks (Settings), never by itself. The file comes
 * from yt-dlp's own release of that version and is checked against the
 * list's SHA-256 when it arrives, and again every time Zajil starts.
 */
object ToolsUpdate {
    private const val LIST =
        "https://github.com/motalim-ai/zajil-download-manager/releases/download/android-channel/tools-android.json"

    /** The list this APK's yt-dlp counts as (tools/sequence-android.txt): only newer ones count. */
    private val BUILT_IN_SEQUENCE = BuildConfig.TOOLS_SEQUENCE.toULong()

    /** yt-dlp's pure-Python release file, which runs in the app's Python. */
    private fun ytDlpUrl(version: String) = "https://github.com/yt-dlp/yt-dlp/releases/download/$version/yt-dlp"

    private const val MAX_LIST = 64 * 1024
    private const val MAX_YT_DLP = 40 * 1024 * 1024

    private fun folder(context: Context) = File(context.filesDir, "tools").apply { mkdirs() }

    @Volatile
    private var checkedThisRun: Pair<AndroidTools, File>? = null

    /** The newer yt-dlp kept, if its list and its file both still check; else none. */
    fun ytDlpZip(context: Context): File? = kept(context)?.second

    private fun kept(context: Context): Pair<AndroidTools, File>? {
        checkedThisRun?.let { return it }
        val dir = folder(context)
        val json = File(dir, "tools-android.json").takeIf { it.isFile } ?: return null
        val signature = File(dir, "tools-android.json.minisig").takeIf { it.isFile } ?: return null
        val tools = runCatching {
            checkAndroidTools(json.readBytes(), signature.readText(), BUILT_IN_SEQUENCE)
        }.getOrNull() ?: return null
        val zip = File(dir, "yt-dlp-${tools.ytDlpVersion}.zip")
        if (!zip.isFile || sha256(zip.readBytes()) != tools.ytDlpSha256) return null
        return (tools to zip).also { checkedThisRun = it }
    }

    /** A list newer than the one in use, or null if there is none. */
    data class Offer(val tools: AndroidTools, val json: ByteArray, val signature: String)

    suspend fun check(context: Context): Result<Offer?> = withContext(Dispatchers.IO) {
        runCatching {
            val json = fetch(LIST, MAX_LIST) ?: return@runCatching null
            val signature = fetch("$LIST.minisig", MAX_LIST)?.toString(Charsets.UTF_8) ?: return@runCatching null
            val newerThan = kept(context)?.first?.sequence ?: BUILT_IN_SEQUENCE
            val tools = try {
                checkAndroidTools(json, signature, newerThan)
            } catch (e: Exception) {
                if ("not newer" in e.message.orEmpty()) return@runCatching null
                throw e
            }
            Offer(tools, json, signature)
        }
    }

    /** Downloads the offered yt-dlp, checks it, and keeps it with its list. Used from the next start. */
    suspend fun apply(context: Context, offer: Offer): Result<Unit> = withContext(Dispatchers.IO) {
        runCatching {
            val bytes = fetch(ytDlpUrl(offer.tools.ytDlpVersion), MAX_YT_DLP) ?: throw IOException("no such yt-dlp")
            if (sha256(bytes) != offer.tools.ytDlpSha256) throw IOException("yt-dlp is not the one the list names")
            val dir = folder(context)
            val zip = File(dir, "yt-dlp-${offer.tools.ytDlpVersion}.zip")
            File(dir, "${zip.name}.tmp").apply { writeBytes(bytes) }.renameTo(zip)
            File(dir, "tools-android.json.tmp").apply { writeBytes(offer.json) }.renameTo(File(dir, "tools-android.json"))
            File(dir, "tools-android.json.minisig.tmp").apply { writeText(offer.signature) }
                .renameTo(File(dir, "tools-android.json.minisig"))
            dir.listFiles { f -> f.name.startsWith("yt-dlp-") && f != zip }?.forEach(File::delete)
            Unit
        }
    }

    private fun fetch(url: String, max: Int) = Https.bytes(url, max)

    private fun sha256(bytes: ByteArray) = Https.sha256(bytes)
}
