package io.github.motalim_ai.zajil.browser

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class IconLinksTest {
    @Test
    fun only_the_sites_own_https_icons_are_asked_never_a_service() {
        val page = "https://m.youtube.com/"
        assertTrue(IconLinks.acceptable("https://www.youtube.com/s/desktop/favicon_144x144.png", page))
        assertFalse(IconLinks.acceptable("http://m.youtube.com/favicon.ico", page))
        assertFalse(IconLinks.acceptable("https://www.google.com/s2/favicons?domain=youtube.com", page))
        assertFalse(IconLinks.acceptable("https://icons.duckduckgo.com/ip3/youtube.com.ico", page))
        assertFalse(IconLinks.acceptable("https://static.xx.fbcdn.net/rsrc.php/icon.png", "https://m.facebook.com/"))
        assertFalse(IconLinks.acceptable("https://u:p@m.youtube.com/favicon.ico", page))
    }

    @Test
    fun declared_icons_come_first_touch_icon_then_by_size_svg_left_out() {
        val html = """
            <link rel="icon" href="/small.png" sizes="16x16">
            <link rel='shortcut icon' href='/big.png' sizes='192x192'>
            <link rel="apple-touch-icon" href="https://m.youtube.com/touch.png?a=1&amp;b=2">
            <link rel="icon" type="image/svg+xml" href="/logo.svg">
            <link rel="mask-icon" href="/mask.png">
            <link rel=stylesheet href=/style.css>
        """
        assertEquals(
            listOf("https://m.youtube.com/touch.png?a=1&b=2", "https://m.youtube.com/big.png", "https://m.youtube.com/small.png"),
            IconLinks.declared("https://m.youtube.com/", html),
        )
        val all = IconLinks.candidates("https://m.youtube.com/", html)
        assertEquals(IconLinks.MAX_TRIES, all.size)
        assertEquals("https://m.youtube.com/touch.png?a=1&b=2", all.first())
        // No page read: the usual places on the site itself.
        assertEquals(
            listOf("https://x.com/apple-touch-icon.png", "https://x.com/favicon.ico"),
            IconLinks.candidates("https://x.com/", null),
        )
    }

    @Test
    fun an_icon_file_is_named_only_by_a_plain_site() {
        assertEquals("youtube.com.png", IconLinks.fileName("youtube.com"))
        assertEquals("127.0.0.1.png", IconLinks.fileName("127.0.0.1"))
        assertNull(IconLinks.fileName("../evil"))
        assertNull(IconLinks.fileName("a/b.com"))
        assertNull(IconLinks.fileName("[::1]"))
        assertNull(IconLinks.fileName(""))
    }
}
