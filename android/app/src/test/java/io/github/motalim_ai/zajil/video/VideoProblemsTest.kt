package io.github.motalim_ai.zajil.video

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class VideoProblemsTest {
    @Test
    fun facebooks_cannot_parse_data_is_a_post_to_sign_in_for_owners_report() {
        val error = "ERROR: [facebook] 1083128824551049: Cannot parse data; please report this issue on  " +
            "https://github.com/yt-dlp/yt-dlp/issues?q= , filling out the appropriate issue template."
        val problem = VideoProblems.of(error, "https://www.facebook.com/reel/1083128824551049")
        assertEquals(VideoProblem.NotPublic, problem)
        assertTrue(problem.signIn)
    }

    @Test
    fun yt_dlps_words_map_to_ours_cause_first() {
        val yt = "https://www.youtube.com/watch?v=x"
        assertEquals(VideoProblem.BotCheck, VideoProblems.of("ERROR: [youtube] x: Sign in to confirm you're not a bot.", yt))
        assertEquals(VideoProblem.Private, VideoProblems.of("ERROR: [youtube] x: Private video. Sign in if you've been granted access", yt))
        assertEquals(VideoProblem.SignIn, VideoProblems.of("ERROR: [instagram] x: login required", "https://www.instagram.com/p/x/"))
        assertEquals(VideoProblem.Gone, VideoProblems.of("ERROR: [youtube] x: Video unavailable", yt))
        assertEquals(VideoProblem.Unsupported, VideoProblems.of("ERROR: Unsupported URL: https://example.com/a", "https://example.com/a"))
        assertEquals(VideoProblem.NoFormats, VideoProblems.of("ERROR: Requested format is not available", yt))
        assertEquals(VideoProblem.Network, VideoProblems.of("ERROR: Unable to download webpage: The read operation timed out", yt))
        assertEquals(VideoProblem.NotPublic, VideoProblems.of("ERROR: [Instagram] x: Instagram sent an empty media response", "https://www.instagram.com/reel/x/"))
        assertEquals(VideoProblem.Unreadable, VideoProblems.of("ERROR: something new", "https://vimeo.com/1"))
        assertFalse(VideoProblem.Unreadable.signIn)
    }

    @Test
    fun the_log_line_has_no_address() {
        val line = VideoProblems.forLog("ERROR: failed https://cdn.example.com/v.mp4?sig=SECRET\nmore")
        assertFalse("SECRET" in line)
        assertFalse("more" in line)
    }
}
