package io.github.motalim_ai.zajil.browser

/** A site the app puts on the browser's home, from presets version [since] on. */
data class Preset(val name: String, val url: String, val since: Int)

/**
 * The browser home's preset sites, and how a home list the user has
 * already changed (kept in the app's preferences) follows when they change.
 */
object ShortcutPresets {
    /** Raised whenever the presets change, so a stored home list follows them once. */
    const val VERSION = 3

    /** The owner's YouTube channel «متعلم» (owner's request and address, 2026-11-01), from presets version 3. */
    const val MOTALIM_CHANNEL = "https://www.youtube.com/@motalim-ai"

    /**
     * Presets the app no longer has, by the version that dropped them
     * (owner's request, 2026-11-01: Vimeo and SoundCloud out). Only the
     * exact address the app put goes: a site the user added stays.
     */
    private val dropped = mapOf("https://vimeo.com/" to 2, "https://m.soundcloud.com/" to 2)

    /** [stored] (presets version [from]) with the presets dropped since, and the new ones added at the end. */
    fun migrate(stored: List<Shortcut>, from: Int, presets: List<Preset>): List<Shortcut> {
        val kept = stored.filterNot { shortcut -> dropped[shortcut.url]?.let { it > from } == true }
        val added = presets.filter { it.since > from && kept.none { k -> k.url == it.url } }.map { Shortcut(it.name, it.url) }
        return kept + added
    }

    /** The addresses [migrate] drops from a list at version [from]. */
    fun droppedSince(from: Int): Set<String> = dropped.filterValues { it > from }.keys
}
