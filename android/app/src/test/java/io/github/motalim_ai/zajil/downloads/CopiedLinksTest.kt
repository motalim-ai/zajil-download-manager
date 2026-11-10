package io.github.motalim_ai.zajil.downloads

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class CopiedLinksTest {
    @Test
    fun aCopyAfterTheLastLookIsNew() {
        assertTrue(CopiedLinks.isNew(copiedAt = 2_000, seenAt = 1_000))
        assertFalse(CopiedLinks.isNew(copiedAt = 1_000, seenAt = 1_000))
        assertFalse(CopiedLinks.isNew(copiedAt = 500, seenAt = 1_000))
    }

    @Test
    fun aPhoneThatGivesNoTimeLooksAtTheTextEveryTime() {
        assertTrue(CopiedLinks.isNew(copiedAt = 0, seenAt = 1_000))
    }

    @Test
    fun theLinkLastOfferedIsNotOfferedAgain() {
        val link = "https://x.com/user/status/1?s=46"
        assertFalse(CopiedLinks.shouldOffer(link, CopiedLinks.hash(link), inList = false))
        assertTrue(CopiedLinks.shouldOffer(link, CopiedLinks.hash("https://example.com/f.zip"), inList = false))
        assertTrue(CopiedLinks.shouldOffer(link, lastOffered = null, inList = false))
    }

    @Test
    fun aLinkAlreadyInTheListIsNotOffered() {
        assertFalse(CopiedLinks.shouldOffer("https://example.com/f.zip", lastOffered = null, inList = true))
    }

    @Test
    fun clipboardHtmlAddressCanBeSuggestedWithoutBeingOfferedAsADownload() {
        assertEquals("https://example.com/index.html", CopiedLinks.suggestion("https://example.com/index.html"))
        assertEquals("https://example.com/file.zip", CopiedLinks.suggestion("Copied file: https://example.com/file.zip"))
    }

    @Test
    fun onlyAHashOfTheLinkIsKept() {
        val link = "https://example.com/f.zip?sig=secret"
        val kept = CopiedLinks.hash(link)
        assertEquals(64, kept.length)
        assertFalse("secret" in kept)
        assertNotEquals(kept, CopiedLinks.hash("https://example.com/f.zip"))
    }
}
