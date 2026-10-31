package io.github.motalim_ai.zajil.video

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class VideoChoiceTest {
    private fun video(id: String, height: Int, vcodec: String, ext: String, tbr: Double = 1000.0, size: Long? = 1000) =
        Format(id, ext, vcodec, "none", height, 30.0, tbr, size, null, null, null, "https://v/$id", null, false)

    private fun sound(
        id: String,
        acodec: String,
        ext: String,
        language: String?,
        note: String?,
        preference: Int? = null,
        tbr: Double = 128.0,
    ) = Format(id, ext, "none", acodec, null, null, tbr, 100, language, preference, note, "https://a/$id", null, false)

    private val own = sound("140", "mp4a.40.2", "m4a", "en", "English original (default), medium", 10)
    private val ownDrc = sound("140-drc", "mp4a.40.2", "m4a", "en", "English original (default), medium, DRC", 10, tbr = 129.0)
    private val ownOpus = sound("251", "opus", "webm", "en", "English original (default), medium", 10, tbr = 140.0)
    private val arabic = sound("140-ar", "mp4a.40.2", "m4a", "ar", "Arabic (dubbed-auto), medium", -1)
    private val arabicOpus = sound("251-ar", "opus", "webm", "ar", "Arabic (dubbed-auto), medium", -1)

    private fun info(vararg formats: Format) = VideoInfo("id", "title", 60.0, false, formats.toList(), "2026.10.21")

    @Test
    fun the_qualities_are_the_heights_that_can_be_joined_highest_first() {
        val i = info(
            video("137", 1080, "avc1.640028", "mp4"),
            video("248", 1080, "vp9", "webm"),
            video("313", 2160, "vp9", "webm"),
            video("399", 1080, "av01.0.08M.08", "mp4"),
            video("701", 2160, "av01.0.12M.10", "mp4"),
            video("134", 360, "avc1.4d401e", "mp4"),
            own,
        )
        // AV1 alone (none here alone) would be left out: MediaMuxer is not asked for it.
        assertEquals(listOf(2160, 1080, 360), VideoChoice.qualities(i))
    }

    @Test
    fun h264_with_aac_in_mp4_comes_first_at_a_height() {
        val i = info(video("137", 1080, "avc1.640028", "mp4", tbr = 3000.0), video("248", 1080, "vp9", "webm", tbr = 2500.0), own, ownOpus)
        val picked = VideoChoice.pick(i, 1080, Sound("en", original = true))!!
        assertEquals("137", picked.video!!.id)
        assertEquals("140", picked.audio.id)
        assertEquals(Container.Mp4, picked.container)
    }

    @Test
    fun vp9_takes_opus_into_webm_when_only_vp9_has_the_height() {
        val i = info(video("313", 2160, "vp9", "webm"), own, ownOpus)
        val picked = VideoChoice.pick(i, 2160, Sound("en", original = true))!!
        assertEquals("251", picked.audio.id)
        assertEquals(Container.Webm, picked.container)
    }

    @Test
    fun the_videos_own_sound_is_offered_first_then_the_dubbed_ones() {
        val i = info(video("137", 1080, "avc1", "mp4"), arabic, own, arabicOpus, ownOpus)
        assertEquals(listOf(Sound("en", true), Sound("ar", false)), VideoChoice.sounds(i))
    }

    @Test
    fun a_dubbed_sound_picks_that_language() {
        val i = info(video("137", 1080, "avc1", "mp4"), arabic, own)
        val picked = VideoChoice.pick(i, 1080, Sound("ar", original = false))!!
        assertEquals("140-ar", picked.audio.id)
        assertEquals(VideoChoice.pick(i, 1080, Sound("en", true))!!.audio.id, "140")
    }

    @Test
    fun the_plain_sound_wins_over_the_range_compressed_one() {
        val i = info(video("137", 1080, "avc1", "mp4"), ownDrc, own)
        assertEquals("140", VideoChoice.pick(i, 1080, Sound("en", true))!!.audio.id)
        assertEquals("140", VideoChoice.pick(i, null, Sound("en", true))!!.audio.id)
    }

    @Test
    fun sound_only_is_m4a() {
        val picked = VideoChoice.pick(info(own, ownOpus), null, Sound("en", true))!!
        assertNull(picked.video)
        assertEquals(Container.M4a, picked.container)
    }

    @Test
    fun no_pair_is_null_and_the_size_adds_both_streams() {
        val i = info(video("313", 2160, "vp9", "webm", size = 900), own)
        // VP9 needs Opus, and there is only AAC.
        assertNull(VideoChoice.pick(i, 2160, Sound("en", true)))
        val j = info(video("137", 1080, "avc1", "mp4", size = 900), own)
        assertEquals(1000L, VideoChoice.pick(j, 1080, Sound("en", true))!!.size)
    }

    @Test
    fun youtube_pages_are_video_pages_and_files_are_not() {
        listOf(
            "https://www.youtube.com/watch?v=jNQXAC9IVRw",
            "https://youtu.be/jNQXAC9IVRw",
            "https://m.youtube.com/shorts/abc",
            "https://music.youtube.com/watch?v=x",
        ).forEach { assertTrue(it, VideoPages.isVideoPage(it)) }
        listOf(
            "https://www.youtube.com/",
            "https://example.com/watch?v=x",
            "https://rr1---sn-x.googlevideo.com/videoplayback?itag=18",
        ).forEach { assertFalse(it, VideoPages.isVideoPage(it)) }
    }

    @Test
    fun a_link_about_to_expire_is_known_by_its_expire() {
        val now = 1_800_000_000L
        assertTrue(VideoPages.expiresSoon("https://r.googlevideo.com/videoplayback?expire=${now + 60}&itag=18", now))
        assertFalse(VideoPages.expiresSoon("https://r.googlevideo.com/videoplayback?expire=${now + 3600}&itag=18", now))
        assertFalse(VideoPages.expiresSoon("https://example.com/file.mp4", now))
    }
}
