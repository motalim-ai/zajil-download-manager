package io.github.motalim_ai.zajil.downloads

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class LinksTest {
    @Test
    fun the_link_in_shared_arabic_text_without_the_sentence_around_it() {
        assertEquals(
            "https://example.com/files/a.zip?x=1",
            Links.first("شوف الملف ده: https://example.com/files/a.zip?x=1، حلو جدًا."),
        )
    }

    @Test
    fun a_link_in_brackets_or_quotes() {
        assertEquals("http://example.com/a", Links.first("(http://example.com/a)"))
        assertEquals("https://example.com/b", Links.first("«https://example.com/b»"))
    }

    @Test
    fun the_first_of_several_links() {
        assertEquals("https://a.example/1", Links.first("https://a.example/1 then https://b.example/2"))
    }

    @Test
    fun no_link_is_null() {
        assertNull(Links.first("نص بلا رابط"))
        assertNull(Links.first("ftp://example.com/file"))
        assertNull(Links.first("http:// and https://"))
    }
}
