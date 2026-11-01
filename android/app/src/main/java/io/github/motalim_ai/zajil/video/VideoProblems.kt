package io.github.motalim_ai.zajil.video

import io.github.motalim_ai.zajil.R

/**
 * What a failure of yt-dlp's comes to, said in the user's language, as
 * the desktop's `ToolProblem` (zajil-core video/messages.rs): yt-dlp's
 * own English words are never shown (owner's report, 2026-11-01:
 * Facebook's «Cannot parse data; please report this issue…»).
 * [signIn]: signing in on the site, in Zajil's browser, may let it read.
 */
enum class VideoProblem(val message: Int, val signIn: Boolean = false) {
    BotCheck(R.string.video_bot_check),
    Live(R.string.video_live),
    Drm(R.string.video_drm),
    Private(R.string.video_private, signIn = true),
    SignIn(R.string.video_sign_in, signIn = true),
    /** A sign-in site whose page yt-dlp could not read: often a post that is not public. */
    NotPublic(R.string.video_not_public, signIn = true),
    Gone(R.string.video_gone),
    Unsupported(R.string.video_unsupported),
    NoFormats(R.string.video_nothing),
    Network(R.string.video_network),
    Unreadable(R.string.video_unreadable),
}

object VideoProblems {
    /** The sites a user signs in to in Zajil's browser for their videos. */
    private val signInSites = Regex("(^|\\.)(facebook\\.com|fb\\.watch|instagram\\.com|x\\.com|twitter\\.com|tiktok\\.com)$")

    /** What yt-dlp's [error] for [url] comes to: its cause before what it caused, as on the desktop. */
    fun of(error: String, url: String): VideoProblem {
        val lower = error.lowercase()
        val says = { phrases: List<String> -> phrases.any { it in lower } }
        val host = runCatching { java.net.URI(url).host }.getOrNull()?.lowercase().orEmpty()
        return when {
            "confirm you" in lower && "bot" in lower -> VideoProblem.BotCheck
            says(listOf("drm protected", "drm-protected", "uses drm")) -> VideoProblem.Drm
            says(listOf("private video", "video is private", "this account is private")) -> VideoProblem.Private
            says(
                listOf(
                    "sign in to confirm", "login required", "log in", "use --cookies", "--cookies-from-browser",
                    "only available for registered users", "members-only", "join this channel", "age-restricted",
                ),
            ) -> VideoProblem.SignIn
            says(listOf("unsupported url")) -> VideoProblem.Unsupported
            // Before «is not available»: yt-dlp's words for no format say it too.
            says(listOf("requested format is not available", "no video formats found")) -> VideoProblem.NoFormats
            says(listOf("video unavailable", "is not available", "has been removed", "no longer available", "http error 404", "not available in your country")) ->
                VideoProblem.Gone
            says(listOf("timed out", "unable to download webpage", "connection", "network is unreachable", "name resolution")) ->
                VideoProblem.Network
            // Facebook's «Cannot parse data», Instagram's «empty media response»: what a
            // post that is not public gives when no one is signed in.
            signInSites.containsMatchIn(host) -> VideoProblem.NotPublic
            else -> VideoProblem.Unreadable
        }
    }

    /** yt-dlp's words for the log: one line, without any address (its query may hold a token). */
    fun forLog(error: String): String =
        error.lineSequence().firstOrNull().orEmpty().replace(Regex("https?://\\S+"), "<url>").take(300)
}
