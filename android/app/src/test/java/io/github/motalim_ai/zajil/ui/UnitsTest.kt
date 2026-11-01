package io.github.motalim_ai.zajil.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class UnitsTest {
    private val ar = SizeUnits("%s بايت", "%s ك.ب", "%s م.ب", "%s ج.ب")

    @Test
    fun sizesAsTheDesignShowsThem() {
        assertEquals("512 بايت", Units.size(512, ar))
        assertEquals("850 ك.ب", Units.size(850L * 1024, ar))
        assertEquals("2.4 م.ب", Units.size((2.4 * 1024 * 1024).toLong(), ar))
        assertEquals("312 م.ب", Units.size(312L shl 20, ar))
        assertEquals("1.2 ج.ب", Units.size((1.2 * (1L shl 30)).toLong(), ar))
    }

    @Test
    fun theUnitIsChosenAfterRounding() {
        // 1,048,575 bytes is «1 MB», never «1024 KB».
        assertEquals("1 م.ب", Units.size((1L shl 20) - 1, ar))
        assertEquals("1 ج.ب", Units.size((1L shl 30) - 1, ar))
    }

    @Test
    fun aWholeNumberHasNoFraction() {
        assertEquals("5 م.ب", Units.size(5L shl 20, ar))
    }

    @Test
    fun hugeAndNegativeSizesDoNotBreak() {
        assertEquals("0 بايت", Units.size(-1, ar))
        Units.size(Long.MAX_VALUE, ar)
    }

    @Test
    fun progressWritesTheUnitOnce() {
        val of = "%1\$s من %2\$s"
        assertEquals("312 من 488 م.ب", Units.progress(312L shl 20, 488L shl 20, ar, of))
        assertEquals("42.1 من 57 م.ب", Units.progress((42.1 * (1 shl 20)).toLong(), 57L shl 20, ar, of))
        assertEquals("0.5 من 1.2 ج.ب", Units.progress(1L shl 29, (1.2 * (1L shl 30)).toLong(), ar, of))
        assertEquals("0 من 850 ك.ب", Units.progress(0, 850L * 1024, ar, of))
    }

    @Test
    fun timeLeft() {
        assertEquals("0:56", Units.left(56L * 1000, 1000))
        assertEquals("12:03", Units.left((12 * 60 + 3) * 1000L, 1000))
        assertEquals("1:02:09", Units.left((3600 + 2 * 60 + 9) * 1000L, 1000))
        assertNull(Units.left(1000, 0))
        assertNull(Units.left(Long.MAX_VALUE, 1))
    }
}
