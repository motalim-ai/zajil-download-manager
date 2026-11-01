package io.github.motalim_ai.zajil.video

import io.github.motalim_ai.zajil.downloads.FileNames
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PlaylistTest {
    @Test
    fun a_listed_playlist_is_read_with_its_entries_bounded_and_checked() {
        val entries = (1..1200).joinToString(",") { """{"url":"https://www.youtube.com/watch?v=$it","title":"V$it","duration":61,"thumbnail":"https://i.ytimg.com/$it.jpg","playlist":false,"available":true}""" }
        val page = Page.parse("""{"playlist":true,"title":"My list","channel":"متعلم","entries":[$entries]}""").getOrThrow() as Page.Playlist
        assertEquals("My list", page.info.title)
        assertEquals(Page.MAX_ENTRIES, page.info.entries.size)
        assertTrue(page.info.entries.all { it.downloadable })
        val odd = Page.parse(
            """{"playlist":true,"title":"x","entries":[
               {"url":"javascript:alert(1)","title":"a"},
               {"url":"https://www.youtube.com/watch?v=p","title":"[Private video]","available":false},
               {"url":"https://www.youtube.com/@x/videos","title":"Videos","playlist":true},
               {"url":"https://www.youtube.com/watch?v=ok","thumbnail":"http://plain/thumb.jpg"}]}""",
        ).getOrThrow() as Page.Playlist
        assertNull(odd.info.entries[0].url)
        assertFalse(odd.info.entries[0].downloadable)
        assertFalse(odd.info.entries[1].downloadable)
        assertFalse(odd.info.entries[2].downloadable)
        assertTrue(odd.info.entries[2].playlist)
        assertTrue(odd.info.entries[3].downloadable)
        assertNull(odd.info.entries[3].thumbnail)
    }

    @Test
    fun a_video_in_a_list_offers_the_list_and_a_list_is_known() {
        assertEquals(
            "https://www.youtube.com/playlist?list=PLwP_SiAcdui0KVebT0mU9Apz359a4ubsC",
            PlaylistLinks.listOf("https://www.youtube.com/watch?v=6ueeeG0TBjs&list=PLwP_SiAcdui0KVebT0mU9Apz359a4ubsC&index=2"),
        )
        assertNull(PlaylistLinks.listOf("https://www.youtube.com/watch?v=6ueeeG0TBjs"))
        assertNull(PlaylistLinks.listOf("https://www.youtube.com/playlist?list=PLx"))
        assertNull(PlaylistLinks.listOf("https://evil.example/watch?v=a&list=PLx"))
        assertNull(PlaylistLinks.listOf("https://www.youtube.com/watch?v=a&list=PL%2F..%2F"))
        listOf(
            "https://www.youtube.com/playlist?list=PLx",
            "https://m.youtube.com/@motalim-ai/videos",
            "https://www.youtube.com/@motalim-ai",
            "https://www.youtube.com/channel/UC123/videos",
        ).forEach {
            assertTrue(it, PlaylistLinks.isList(it))
            assertTrue(it, VideoPages.isVideoPage(it))
        }
        assertFalse(PlaylistLinks.isList("https://www.youtube.com/watch?v=a"))
        assertFalse(VideoPages.isVideoPage("https://www.youtube.com/@motalim-ai/community"))
    }

    @Test
    fun a_playlists_folder_name_is_safe() {
        assertEquals("متعلم - MOTALIM - Videos", FileNames.folder("متعلم - MOTALIM - Videos"))
        assertEquals("a b", FileNames.folder("../a/\\ b‮"))
        assertEquals("", FileNames.folder("..."))
        assertEquals(80, FileNames.folder("x".repeat(200)).length)
    }
}
