package io.github.motalim_ai.zajil.browser

import org.junit.Assert.assertEquals
import org.junit.Test

class BlockedTodayTest {
    @Test
    fun countsEachListOnItsDay() {
        val t = BlockedToday().plus(ListKind.Ads, 100).plus(ListKind.Ads, 100).plus(ListKind.Trackers, 100).plus(ListKind.Malware, 100)
        assertEquals(BlockedToday(100, ads = 2, trackers = 1, malware = 1), t)
        assertEquals(4L, t.total)
    }

    @Test
    fun aNewDayStartsFromZero() {
        val yesterday = BlockedToday(99, ads = 5, trackers = 3, malware = 1)
        assertEquals(BlockedToday(100), yesterday.on(100))
        assertEquals(BlockedToday(100, malware = 1), yesterday.plus(ListKind.Malware, 100))
        assertEquals(yesterday, yesterday.on(99))
    }

    @Test
    fun aHugeCountStops() {
        val full = BlockedToday(1, ads = Int.MAX_VALUE, trackers = Int.MAX_VALUE, malware = Int.MAX_VALUE)
        assertEquals(full, full.plus(ListKind.Ads, 1))
        assertEquals(3L * Int.MAX_VALUE, full.total)
    }
}
