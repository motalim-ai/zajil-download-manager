package io.github.motalim_ai.zajil.browser

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.util.zip.GZIPOutputStream
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class HostListTest {
    private fun list(vararg lines: String): HostList {
        val out = ByteArrayOutputStream()
        GZIPOutputStream(out).use { it.write(("# zajil-hosts-1 test\n" + lines.joinToString("\n")).toByteArray()) }
        return HostList.read(ByteArrayInputStream(out.toByteArray()))
    }

    @Test
    fun a_host_and_what_is_under_it_match_and_a_kept_one_does_not() {
        val ads = list("doubleclick.net", "ads.example.com", "@@safe.ads.example.com")
        assertTrue(ads.matches("doubleclick.net"))
        assertTrue(ads.matches("stats.g.doubleclick.net"))
        assertTrue(ads.matches("ads.example.com"))
        assertTrue(ads.matches("x.ads.example.com"))
        assertFalse(ads.matches("example.com"))
        assertFalse(ads.matches("safe.ads.example.com"))
        assertFalse(ads.matches("notdoubleclick.net"))
        assertFalse(ads.matches("net"))
        assertEquals(2, ads.size)
    }

    @Test
    fun an_empty_list_matches_nothing() {
        assertFalse(HostList.EMPTY.matches("doubleclick.net"))
        assertFalse(list().matches("anything.com"))
    }

    @Test
    fun the_shipped_lists_are_read_and_match() {
        val dir = java.io.File("src/main/assets/blocklists")
        // A «.gz» asset is unpacked by the build and loses its ending: the app would read nothing.
        assertTrue(dir.list()!!.none { it.endsWith(".gz") })
        val lists = ListKind.entries.associateWith { kind -> dir.resolve("${kind.file}.hosts").inputStream().use(HostList::read) }
        val ads = lists.getValue(ListKind.Ads)
        val malware = lists.getValue(ListKind.Malware)
        assertTrue(lists.values.all { it.size > 100 })
        assertTrue(ads.size > 10_000)
        assertTrue(ads.matches("doubleclick.net"))
        assertTrue(malware.size > 100)
        assertFalse(malware.matches("www.youtube.com"))
        assertFalse(ads.matches("www.wikipedia.org"))
    }

    @Test
    fun youtubes_ads_are_let_through_by_default() {
        val s = GuardSettings()
        assertFalse(s.active(Guard.Ads, "youtube.com"))
        assertTrue(s.active(Guard.Malware, "youtube.com"))
        assertTrue(s.active(Guard.Ads, "example.com"))
        assertFalse(s.copy(on = setOf(Guard.Malware)).active(Guard.Ads, "example.com"))
    }
}
