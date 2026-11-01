package io.github.motalim_ai.zajil.browser

import java.net.URI

/**
 * Where a site's own icon is, for the browser's home (owner's decision,
 * 2026-11-01): from that site only, over https, never a favicon service
 * and never a logo in the APK. Everything read here is untrusted.
 */
object IconLinks {
    /** The most read of a page or an icon. */
    const val MAX_BYTES = 256 * 1024

    /** The most icons asked for, per site. */
    const val MAX_TRIES = 4

    /** [site]'s icon file, if [site] is a plain host name (as [Finds.site] gives). */
    fun fileName(site: String): String? =
        site.takeIf { it.length <= 253 && Regex("^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)*$").matches(it) }
            ?.let { "$it.png" }

    /** Whether Zajil may ask [url] for [page]'s icon: https, and [page]'s own site. */
    fun acceptable(url: String, page: String): Boolean {
        val uri = runCatching { URI(url) }.getOrNull() ?: return false
        return uri.scheme.equals("https", ignoreCase = true) && !uri.host.isNullOrEmpty() && uri.userInfo == null &&
            Finds.sameSite(url, page)
    }

    /**
     * The icons to try for [page], best first: those its HTML declares
     * (`apple-touch-icon`, then `icon` by size), then the usual
     * `/apple-touch-icon.png` and `/favicon.ico`; only [acceptable] ones.
     */
    fun candidates(page: String, html: String?): List<String> {
        val origin = runCatching { URI(page) }.getOrNull()?.let { "https://${it.host}" } ?: return emptyList()
        return (declared(page, html.orEmpty()) + listOf("$origin/apple-touch-icon.png", "$origin/favicon.ico"))
            .distinct()
            .filter { acceptable(it, page) }
            .take(MAX_TRIES)
    }

    /** The `<link rel=…icon…>` of [html], resolved against [page], best first. SVG is left out (Android does not decode it). */
    fun declared(page: String, html: String): List<String> {
        val base = runCatching { URI(page) }.getOrNull() ?: return emptyList()
        data class Link(val href: String, val touch: Boolean, val size: Int)
        val links = Regex("<link\\b[^>]*>", RegexOption.IGNORE_CASE).findAll(html.take(MAX_BYTES)).mapNotNull { tag ->
            val attrs = attributes(tag.value)
            val rel = attrs["rel"]?.lowercase()?.split(Regex("\\s+")) ?: return@mapNotNull null
            val touch = rel.any { it.startsWith("apple-touch-icon") }
            if (!touch && "icon" !in rel) return@mapNotNull null
            val href = attrs["href"]?.replace("&amp;", "&")?.trim()?.takeIf { it.isNotEmpty() } ?: return@mapNotNull null
            if (attrs["type"]?.contains("svg", ignoreCase = true) == true || href.substringBefore('?').endsWith(".svg", ignoreCase = true)) {
                return@mapNotNull null
            }
            val url = runCatching { base.resolve(href).toString() }.getOrNull() ?: return@mapNotNull null
            val size = Regex("(\\d{1,4})x\\d{1,4}").find(attrs["sizes"].orEmpty())?.groupValues?.get(1)?.toIntOrNull() ?: 0
            Link(url, touch, size)
        }
        return links.sortedWith(compareByDescending<Link> { it.touch }.thenByDescending { it.size }).map { it.href }.toList()
    }

    private fun attributes(tag: String): Map<String, String> =
        Regex("([a-zA-Z-]+)\\s*=\\s*(\"([^\"]*)\"|'([^']*)'|([^\\s>]+))").findAll(tag).associate { m ->
            m.groupValues[1].lowercase() to (m.groups[3]?.value ?: m.groups[4]?.value ?: m.groups[5]?.value.orEmpty())
        }
}
