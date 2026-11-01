package io.github.motalim_ai.zajil.browser

import android.content.Context
import io.github.motalim_ai.zajil.BuildConfig
import io.github.motalim_ai.zajil.engine.HostCheck
import io.github.motalim_ai.zajil.engine.checkAndroidLists
import io.github.motalim_ai.zajil.update.Https
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/** What the protection does, each with its own switch (0.3.0, the owner's decision). */
enum class Guard {
    /** Ads and trackers in pages: their requests are not made. */
    Ads,

    /**
     * Sites that spread malware: pages warn, downloads are refused. Phishing
     * has no list of Zajil's (the owner's decision: the open one's sources
     * have terms of their own); WebView's Safe Browsing, always on, warns
     * of it.
     */
    Malware,
}

/** The switches, all on unless the user turns one off, and the sites where the user turned one off. */
data class GuardSettings(
    val on: Set<Guard> = Guard.entries.toSet(),
    /** Site ([Finds.site]) → the guards turned off there. */
    val off: Map<String, Set<Guard>> = DEFAULT_OFF,
) {
    fun active(guard: Guard, site: String?) = guard in on && (site == null || guard !in off[site].orEmpty())

    companion object {
        /**
         * YouTube shows a wall to some ad blockers: its ads are not blocked
         * by default there (the owner's decision). Its malware check stays
         * on.
         */
        val DEFAULT_OFF = mapOf("youtube.com" to setOf(Guard.Ads))
    }
}

/**
 * Zajil's protection in its browser and its downloads (ADR-0012, section
 * 17). Not an antivirus: it protects browsing and downloads inside Zajil
 * only. The lists are open filter lists compiled to hosts, shipped in the
 * APK and updated only from the signed `lists-android.json`; every match
 * is made on the phone, and no address is sent anywhere. (WebView's Safe
 * Browsing, on as always, is Google's own check, with hashed addresses.)
 */
class Protection(private val context: Context) {
    private val prefs = context.getSharedPreferences("protection", Context.MODE_PRIVATE)
    private val io = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val folder = File(context.filesDir, "blocklists")

    private val _settings = MutableStateFlow(readSettings())
    val settings: StateFlow<GuardSettings> = _settings.asStateFlow()

    @Volatile
    private var lists: Map<ListKind, HostList> = emptyMap()

    private val _sequence = MutableStateFlow(0L)

    /** The lists' version: when they were compiled (seconds since 1970); 0 until they are read. */
    val sequence: StateFlow<Long> = _sequence.asStateFlow()

    init {
        io.launch { load() }
    }

    /** What [host] is listed as, for [guard]: ads or trackers, or malware. */
    fun listed(guard: Guard, host: String): Boolean {
        val l = lists
        return when (guard) {
            Guard.Ads -> l[ListKind.Ads]?.matches(host) == true || l[ListKind.Trackers]?.matches(host) == true
            Guard.Malware -> l[ListKind.Malware]?.matches(host) == true
        }
    }

    /** The harmful kind of [host] on [site]'s page with the switches as they are, or null. */
    fun harm(host: String, site: String?): Guard? =
        Guard.Malware.takeIf { _settings.value.active(it, site) && listed(it, host) }

    /** For the engine: a download, or a redirect of one, to a harmful host is refused. */
    val hostCheck = object : HostCheck {
        override fun refused(host: String): Boolean = harm(host, Finds.site(host)) != null
    }

    fun set(guard: Guard, on: Boolean) = save { s -> s.copy(on = if (on) s.on + guard else s.on - guard) }

    /** [guard] on or off on [site] only. */
    fun setFor(site: String, guard: Guard, on: Boolean) = save { s ->
        val now = if (on) s.off[site].orEmpty() - guard else s.off[site].orEmpty() + guard
        s.copy(off = if (now.isEmpty()) s.off - site else s.off + (site to now))
    }

    /** Every exception goes: all guards on everywhere they are on. */
    fun clearExceptions() = save { it.copy(off = emptyMap()) }

    private fun save(how: (GuardSettings) -> GuardSettings) {
        synchronized(this) {
            val s = how(_settings.value)
            prefs.edit()
                .putStringSet(ON, s.on.map { it.name }.toSet())
                .putString(OFF, JSONArray(s.off.map { (site, guards) -> JSONObject().put("site", site).put("off", JSONArray(guards.map { it.name })) }).toString())
                .apply()
            _settings.value = s
        }
    }

    private fun readSettings(): GuardSettings {
        val on = prefs.getStringSet(ON, null)?.mapNotNull { n -> Guard.entries.firstOrNull { it.name == n } }?.toSet()
        val off = prefs.getString(OFF, null)?.let { text ->
            runCatching {
                JSONArray(text).let { a ->
                    (0 until a.length()).associate { i ->
                        val o = a.getJSONObject(i)
                        val guards = o.getJSONArray("off").let { g -> (0 until g.length()).mapNotNull { j -> Guard.entries.firstOrNull { it.name == g.getString(j) } } }
                        o.getString("site") to guards.toSet()
                    }
                }
            }.getOrNull()
        }
        return GuardSettings(on ?: Guard.entries.toSet(), off ?: GuardSettings.DEFAULT_OFF)
    }

    /**
     * The newest lists the phone has: the updated ones if they are newer
     * than the APK's. The APK's are gzip files named `<list>.hosts`: the
     * build unpacks a «.gz» asset and takes its ending off.
     */
    private fun load() {
        val bundled = runCatching { context.assets.open("blocklists/sequence.txt").bufferedReader().use { it.readText().trim().toLong() } }.getOrDefault(0)
        val updated = prefs.getLong(SEQUENCE, 0)
        val useUpdated = updated > bundled && ListKind.entries.all { File(folder, "${it.file}.txt.gz").isFile }
        lists = ListKind.entries.associateWith { kind ->
            runCatching {
                (if (useUpdated) File(folder, "${kind.file}.txt.gz").inputStream() else context.assets.open("blocklists/${kind.file}.hosts"))
                    .use(HostList::read)
            }.getOrDefault(HostList.EMPTY)
        }
        _sequence.value = if (useUpdated) updated else bundled
    }

    /**
     * Asks for newer lists: the signed `lists-android.json` on
     * android-channel, checked with the project's keys, then each list,
     * checked against its signed SHA-256 and size. True if new ones came.
     */
    suspend fun update(): Result<Boolean> = withContext(Dispatchers.IO) {
        runCatching {
            val base = BuildConfig.UPDATE_BASE.ifEmpty { RELEASES }
            val json = Https.bytes("$base/android-channel/lists-android.json", 16 * 1024) ?: return@runCatching false
            val signature = Https.bytes("$base/android-channel/lists-android.json.minisig", 4 * 1024) ?: return@runCatching false
            val signed = try {
                checkAndroidLists(json, String(signature, Charsets.UTF_8), _sequence.value.toULong())
            } catch (e: Exception) {
                // Not newer, or not signed: nothing changes.
                if ((e.message ?: "").contains("not newer")) return@runCatching false
                throw e
            }
            folder.mkdirs()
            for (list in signed.lists) {
                val kind = ListKind.entries.firstOrNull { it.file == list.name } ?: continue
                val temp = File(folder, "${kind.file}.txt.gz.new")
                val sha = Https.toFile("$base/android-channel/${list.file}", temp, list.size.toLong())
                if (sha != list.sha256 || temp.length() != list.size.toLong()) {
                    temp.delete()
                    error("a list does not match its signed SHA-256")
                }
            }
            for (list in signed.lists) {
                val kind = ListKind.entries.firstOrNull { it.file == list.name } ?: continue
                File(folder, "${kind.file}.txt.gz.new").renameTo(File(folder, "${kind.file}.txt.gz"))
            }
            prefs.edit().putLong(SEQUENCE, signed.sequence.toLong()).apply()
            load()
            true
        }
    }

    private companion object {
        const val ON = "on"
        const val OFF = "off"
        const val SEQUENCE = "lists_sequence"
        const val RELEASES = "https://github.com/motalim-ai/zajil-download-manager/releases/download"
    }
}
