package io.github.motalim_ai.zajil.downloads

import android.content.ClipboardManager
import android.content.Context
import java.security.MessageDigest

/**
 * A link the user copied, read only while a Zajil window has the focus
 * (Android 10 and later allow nothing else): never in the background.
 *
 * The browser may show a copied link as an inline suggestion. A download
 * offer is read only when the user explicitly asks for the clipboard link.
 */
object CopiedLinks {
    /** What the clipboard holds, for [read]. */
    sealed interface Copied {
        /** Android said nothing: no clip, or not yet readable (some phones, just after the focus). */
        data object Unavailable : Copied

        /** Already looked at, or already offered, or already in the list. */
        data object Seen : Copied

        /** No http(s) link in it. */
        data object NoLink : Copied

        data class Link(val url: String) : Copied
    }

    /**
     * Whether a copy is new: its time when the phone gives one (some give 0,
     * then every read looks at the text), and its link not the one last
     * offered nor one already in the list.
     */
    fun isNew(copiedAt: Long, seenAt: Long): Boolean = copiedAt == 0L || copiedAt > seenAt

    fun shouldOffer(link: String, lastOffered: String?, inList: Boolean): Boolean =
        !inList && hash(link) != lastOffered

    /** A link to suggest in the browser, without saving it or creating a download offer. */
    fun suggest(context: Context): String? {
        val clipboard = context.getSystemService(ClipboardManager::class.java) ?: return null
        val text = clipboard.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context)
            ?: return null
        return suggestion(text.toString())
    }

    fun suggestion(text: String): String? = Links.first(text)

    /**
     * The copied link for a surface the user explicitly opened, such as the
     * Quick Settings tile or the home-screen clipboard button.
     */
    fun read(context: Context, onlyNew: Boolean, inList: (String) -> Boolean): Copied {
        val clipboard = context.getSystemService(ClipboardManager::class.java) ?: return Copied.Unavailable
        val description = clipboard.primaryClipDescription ?: return Copied.Unavailable
        val prefs = context.getSharedPreferences("clipboard", Context.MODE_PRIVATE)
        val copiedAt = description.timestamp
        if (onlyNew && !isNew(copiedAt, prefs.getLong(SEEN, 0))) return Copied.Seen
        // Any kind of text: a browser's copy may be HTML with the link in it.
        val text = clipboard.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context)
            ?: return Copied.Unavailable
        prefs.edit().putLong(SEEN, copiedAt).apply()
        val link = Links.first(text.toString()) ?: return Copied.NoLink
        if (onlyNew && !shouldOffer(link, prefs.getString(LAST, null), inList(link))) return Copied.Seen
        prefs.edit().putString(LAST, hash(link)).apply()
        return Copied.Link(link)
    }

    fun hash(link: String): String =
        MessageDigest.getInstance("SHA-256").digest(link.toByteArray()).joinToString("") { "%02x".format(it) }

    private const val SEEN = "seen_at"
    private const val LAST = "last_offered"
}
