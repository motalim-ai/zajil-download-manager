package io.github.motalim_ai.zajil.downloads

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class SpeedometerTest {
    private var now = 0L
    private val meter = Speedometer { now }

    @Test
    fun theFirstReportHasNoSpeed() {
        meter.report(1, 1000)
        assertNull(meter.rates.value[1])
    }

    @Test
    fun measuresOverHalfASecondAtLeast() {
        meter.report(1, 0)
        now = 200
        meter.report(1, 1_000_000)
        assertNull(meter.rates.value[1])
        now = 1000
        meter.report(1, 1_000_000)
        assertEquals(1_000_000L, meter.rates.value[1])
    }

    @Test
    fun smoothsAJump() {
        meter.report(1, 0)
        now = 1000
        meter.report(1, 1_000_000)
        now = 2000
        meter.report(1, 4_000_000)
        // 0.6 × 1 MB/s + 0.4 × 3 MB/s.
        assertEquals(1_800_000L, meter.rates.value[1])
    }

    @Test
    fun startingOverForgetsTheSpeed() {
        meter.report(1, 0)
        now = 1000
        meter.report(1, 1_000_000)
        now = 2000
        meter.report(1, 0)
        now = 3000
        meter.report(1, 500_000)
        assertEquals(500_000L, meter.rates.value[1])
    }

    @Test
    fun forgetsWhatStopped() {
        meter.report(1, 0)
        meter.report(2, 0)
        now = 1000
        meter.report(1, 1000)
        meter.report(2, 2000)
        meter.keepOnly(setOf(2))
        assertEquals(mapOf(2L to 2000L), meter.rates.value)
    }
}
