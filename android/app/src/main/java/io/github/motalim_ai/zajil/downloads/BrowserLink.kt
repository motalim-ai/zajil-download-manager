package io.github.motalim_ai.zajil.downloads

/** Whose cookies go with a download found in the in-app browser (A6). */
enum class CookieSource {
    None,

    /** The browser's own (the user may have signed in there). */
    Browser,

    /** Private mode's: gone when it closes, so never after a restart. */
    Private,
}

/**
 * Where a link was found in the in-app browser: [url], the link it was
 * made for; its page (sent as the browsers send a Referer, without its
 * fragment or any user name and password), whose cookies go with it, and
 * the browser's User-Agent (some servers tie their cookies to it).
 * [cookies] is the `Cookie` header the browser had for [url] when the user
 * tapped: kept in memory only, never in the list on disk (as on the
 * desktop), and given to [url] only. [source] is [CookieSource.None] for a
 * link of another site than the page's.
 */
data class BrowserLink(
    val url: String,
    val page: String,
    val source: CookieSource,
    val userAgent: String?,
    val cookies: String?,
)
