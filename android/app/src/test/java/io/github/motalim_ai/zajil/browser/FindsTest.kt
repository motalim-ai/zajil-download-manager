package io.github.motalim_ai.zajil.browser

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class FindsTest {
    @Test
    fun mediaRequestsByTheirExtension() {
        assertEquals(MediaKind.File, Finds.kindOf("https://cdn.example.com/v/clip.mp4?token=1"))
        assertEquals(MediaKind.File, Finds.kindOf("https://cdn.example.com/a/song.M4A"))
        assertEquals(MediaKind.Hls, Finds.kindOf("https://live.example.com/master.m3u8"))
        assertEquals(MediaKind.Dash, Finds.kindOf("https://cdn.example.com/manifest.mpd"))
        assertNull(Finds.kindOf("https://example.com/page.html"))
        assertNull(Finds.kindOf("https://cdn.example.com/seg-001.ts"))
        assertNull(Finds.kindOf("blob:https://example.com/1234"))
        assertNull(Finds.kindOf("file:///sdcard/a.mp4"))
        // A query that only looks like a file is not one.
        assertNull(Finds.kindOf("https://example.com/watch?f=a.mp4"))
    }

    @Test
    fun renditionsAsTheExtensionKnowsThem() {
        assertTrue(Finds.isRendition("https://video.twimg.com/ext_tw_video/1/pu/pl/mp4a/128000/a.m3u8"))
        assertTrue(Finds.isRendition("https://video.twimg.com/ext_tw_video/1/pu/pl/avc1/1280x720/b.m3u8"))
        assertFalse(Finds.isRendition("https://video.twimg.com/ext_tw_video/1/pu/pl/main.m3u8"))
    }

    private val token = "abc123"

    @Test
    fun aReportWithThisTokenIsRead() {
        val (title, videos) = Finds.parseReport(
            """{"t":"abc123","title":"A page","videos":[{"src":"https://cdn.example.com/v.mp4","w":640,"h":360,"d":75,"p":false}]}""",
            token,
        )!!
        assertEquals("A page", title)
        assertEquals(listOf(PageVideo("https://cdn.example.com/v.mp4", 640, 360, 75, false)), videos)
    }

    @Test
    fun anotherScriptsMessageIsNotAReport() {
        assertNull(Finds.parseReport("""{"t":"guess","videos":[]}""", token))
        assertNull(Finds.parseReport("""{"videos":[]}""", token))
        assertNull(Finds.parseReport("not json", token))
        assertNull(Finds.parseReport(null, token))
        assertNull(Finds.parseReport("x".repeat(Finds.MAX_REPORT + 1), token))
    }

    @Test
    fun aReportsFieldsAreBoundedAndOnlyHttpAddressesKept() {
        val many = (1..50).joinToString(",") { """{"src":"javascript:alert($it)","w":99999999,"h":-5,"d":1e12,"p":true}""" }
        val (title, videos) = Finds.parseReport("""{"t":"abc123","title":"${"t".repeat(500)}\u0007","videos":[$many]}""", token)!!
        assertEquals(200, title.length)
        assertEquals(Finds.MAX_VIDEOS, videos.size)
        videos.forEach {
            assertEquals("", it.src)
            assertEquals(100_000, it.width)
            assertEquals(0, it.height)
            // Out of range: clamped, or read as unknown; never more.
            assertTrue(it.duration in 0..10_000_000)
        }
    }

    @Test
    fun cookiesGoOnlyToThePagesOwnSite() {
        assertTrue(Finds.sameSite("https://www.facebook.com/reel/1", "https://m.facebook.com/"))
        assertTrue(Finds.sameSite("https://video.example.co.uk/a.mp4", "https://www.example.co.uk/page"))
        assertFalse(Finds.sameSite("https://cdn.other.com/a.mp4", "https://www.example.com/page"))
        assertFalse(Finds.sameSite("https://example.co.uk/a.mp4", "https://other.co.uk/page"))
        assertEquals("example.com", Finds.site("a.b.example.com"))
        assertEquals("127.0.0.1", Finds.site("127.0.0.1"))
        assertFalse(Finds.sameSite("http://127.0.0.1/a.mp4", "http://10.0.0.1/page"))
    }

    @Test
    fun theAddressFieldOpensAnAddressOrSearches() {
        assertEquals("https://example.com/x", Finds.addressOf("https://example.com/x"))
        assertEquals("https://example.com/x", Finds.addressOf("example.com/x"))
        assertEquals("https://duckduckgo.com/?q=%D9%81%D9%8A%D8%AF%D9%8A%D9%88+%D9%82%D8%B5%D9%8A%D8%B1", Finds.addressOf("فيديو قصير"))
        assertEquals("https://duckduckgo.com/?q=javascript%3Aalert%281%29", Finds.addressOf("javascript:alert(1)"))
        assertNull(Finds.addressOf("   "))
    }

    @Test
    fun aPageIsListedOnceInTheRecentList() {
        val reel = Visit("A reel", "https://m.facebook.com/reel/1?s=a")
        val other = Visit("Other", "https://m.facebook.com/reel/2")
        val list = Finds.withVisit(listOf(other, reel), Visit("A reel", "https://m.facebook.com/reel/1?s=b"), 8)
        assertEquals(listOf("https://m.facebook.com/reel/1?s=b", "https://m.facebook.com/reel/2"), list.map { it.url })
        // The same title on another host is another page; an untitled one is known by its address only.
        assertEquals(3, Finds.withVisit(list, Visit("A reel", "https://www.instagram.com/reel/1"), 8).size)
        assertEquals(2, Finds.withVisit(listOf(Visit("", "https://a.com/1")), Visit("", "https://a.com/2"), 8).size)
        assertEquals(1, Finds.withVisit(list, Visit("New", "https://a.com/"), 1).size)
    }
}
