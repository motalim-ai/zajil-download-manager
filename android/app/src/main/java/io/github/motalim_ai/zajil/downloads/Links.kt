package io.github.motalim_ai.zajil.downloads

/** Links in text that another app shared, or that is on the clipboard. */
object Links {
    private val link = Regex("""https?://[^\s<>"'«»“”]+""", RegexOption.IGNORE_CASE)

    /** Punctuation that ends a sentence, not the link: "see https://x/y." */
    private const val TRAILING = ".,;:!?،؛؟)]}'\""

    /**
     * The first http(s) link in [text], without the punctuation of the
     * sentence around it; null if there is none. Only a link with a host
     * counts.
     */
    fun first(text: String): String? = link.findAll(text)
        .map { it.value.trimEnd { c -> c in TRAILING } }
        .firstOrNull(::hasHost)

    private fun hasHost(url: String): Boolean {
        val rest = url.substringAfter("://")
        val host = rest.substringBefore('/').substringBefore('?').substringBefore('#').substringAfterLast('@')
        return host.isNotEmpty() && host != ":" && !host.startsWith(':')
    }
}
