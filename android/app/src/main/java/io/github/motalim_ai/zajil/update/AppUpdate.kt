package io.github.motalim_ai.zajil.update

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings
import io.github.motalim_ai.zajil.BuildConfig
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.engine.AndroidUpdate
import io.github.motalim_ai.zajil.engine.checkAndroidUpdate
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext

/**
 * Zajil for Android's own update (ADR-0012, section 8; A4), as ADR-0011
 * does it on the desktop: `latest-android.json` on the `android-channel`
 * pre-release, signed by one of the project's two keys and checked in Rust;
 * the APK of that version from that version's own release, checked against
 * its SHA-256; then Android's installer, which asks the user and checks
 * that the APK is signed by the same key as the installed Zajil. Never
 * silent: the user taps «Update», then «Install» in Android's own dialog.
 */
object AppUpdate {
    private const val RELEASES = "https://github.com/motalim-ai/zajil-download-manager/releases/download"

    /** The project's releases; another address only in a debug build's test. */
    private val base: String get() = BuildConfig.UPDATE_BASE.ifEmpty { RELEASES }

    sealed interface State {
        data object Idle : State
        data object Checking : State
        data object Latest : State
        data class Offered(val update: AndroidUpdate) : State
        data class Downloading(val update: AndroidUpdate, val percent: Int) : State
        data class Ready(val update: AndroidUpdate, val apk: File) : State
        data class Failed(val message: Int) : State
    }

    private val _state = MutableStateFlow<State>(State.Idle)
    val state: StateFlow<State> = _state.asStateFlow()

    /** Asks for the update file; [quiet] (the check at start) says nothing unless there is one. */
    suspend fun check(quiet: Boolean = false) = withContext(Dispatchers.IO) {
        if (!quiet) _state.value = State.Checking
        _state.value = runCatching {
            val json = Https.bytes("$base/android-channel/latest-android.json", 64 * 1024)
            val signature = Https.bytes("$base/android-channel/latest-android.json.minisig", 8 * 1024)
            if (json == null || signature == null) return@runCatching State.Latest
            try {
                State.Offered(
                    checkAndroidUpdate(
                        json,
                        signature.toString(Charsets.UTF_8),
                        BuildConfig.VERSION_CODE.toULong(),
                        BuildConfig.UPDATE_TEST_KEY.ifEmpty { null },
                    ),
                )
            } catch (e: Exception) {
                if ("not newer" in e.message.orEmpty()) State.Latest else State.Failed(R.string.update_unverified)
            }
        }.getOrElse { State.Failed(R.string.update_failed) }
            .let { if (quiet && it !is State.Offered) State.Idle else it }
    }

    /** Downloads the offered APK and checks it against the signed SHA-256. */
    suspend fun download(context: Context, update: AndroidUpdate) = withContext(Dispatchers.IO) {
        val folder = File(context.cacheDir, "update").apply { deleteRecursively(); mkdirs() }
        val apk = File(folder, update.apk)
        _state.value = State.Downloading(update, 0)
        _state.value = runCatching {
            val sha = Https.toFile("$base/android-v${update.version}/${update.apk}", apk, update.size.toLong()) { done ->
                _state.value = State.Downloading(update, (done * 100 / update.size.toLong()).toInt().coerceIn(0, 100))
            }
            if (sha == null || sha != update.sha256 || apk.length() != update.size.toLong()) {
                apk.delete()
                State.Failed(R.string.update_unverified)
            } else {
                State.Ready(update, apk)
            }
        }.getOrElse { e ->
            apk.delete()
            // Longer than its signed size: not the file the update names.
            State.Failed(if (e is Https.TooLong) R.string.update_unverified else R.string.update_failed)
        }
    }

    /**
     * Hands the checked APK to Android's installer, which shows its own
     * dialog. False if Zajil may not ask to install yet: the settings page
     * where the user allows it opens instead.
     */
    fun install(context: Context, apk: File): Boolean {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && !context.packageManager.canRequestPackageInstalls()) {
            context.startActivity(
                Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}"))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
            return false
        }
        val installer = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
            .apply { setAppPackageName(context.packageName) }
        val id = installer.createSession(params)
        installer.openSession(id).use { session ->
            session.openWrite("zajil.apk", 0, apk.length()).use { out ->
                apk.inputStream().use { it.copyTo(out) }
                session.fsync(out)
            }
            // The installer fills in the result, so the intent is mutable.
            val flags = PendingIntent.FLAG_UPDATE_CURRENT or
                (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0)
            val result = PendingIntent.getBroadcast(context, id, Intent(context, UpdateReceiver::class.java), flags)
            session.commit(result.intentSender)
        }
        return true
    }

    /** Android's installer said no (or the user did). */
    internal fun installFailed() {
        _state.value = State.Failed(R.string.update_not_installed)
    }
}
