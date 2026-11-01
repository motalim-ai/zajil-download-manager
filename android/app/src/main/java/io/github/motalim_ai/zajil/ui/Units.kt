package io.github.motalim_ai.zajil.ui

import java.text.DecimalFormat
import java.text.DecimalFormatSymbols
import java.util.Locale

/**
 * Sizes, speeds and times as the desktop shows them (apps/desktop/src/lib
 * format.ts), with the phone's short units («م.ب») and Western digits.
 */
data class SizeUnits(val bytes: String, val kib: String, val mib: String, val gib: String)

object Units {
    private const val KIB = 1024.0
    private const val MIB = 1024 * KIB
    private const val GIB = 1024 * MIB

    private fun number(value: Double, fraction: Int): String {
        val pattern = if (fraction == 0) "0" else "0." + "#".repeat(fraction)
        return DecimalFormat(pattern, DecimalFormatSymbols(Locale.ROOT)).format(value)
    }

    /**
     * A size, its unit chosen after rounding, so 1,048,575 bytes is «1 م.ب»,
     * never «1024 ك.ب». [units] are patterns with one `%s`.
     */
    fun size(bytes: Long, units: SizeUnits): String {
        val b = bytes.coerceAtLeast(0).toDouble()
        fun rounded(unit: Double, fraction: Int): Double {
            val scale = Math.pow(10.0, fraction.toDouble())
            return Math.round(b / unit * scale) / scale
        }
        val mibFraction = if (b >= 100 * MIB) 0 else 1
        return when {
            b >= GIB || rounded(MIB, mibFraction) >= 1024 -> units.gib.format(number(b / GIB, 1))
            b >= MIB || rounded(KIB, 0) >= 1024 -> units.mib.format(number(b / MIB, mibFraction))
            b >= KIB -> units.kib.format(number(b / KIB, 0))
            else -> units.bytes.format(number(b, 0))
        }
    }

    /**
     * [done] of [total] with the unit once, as the design writes it:
     * «312 من 488 م.ب». [ofTotal] has `%1$s` for the done part and `%2$s`
     * for the total with its unit.
     */
    fun progress(done: Long, total: Long, units: SizeUnits, ofTotal: String): String {
        val whole = size(total, units)
        val unit = listOf(units.gib, units.mib, units.kib, units.bytes).first { pattern ->
            val (before, after) = pattern.split("%s", limit = 2).let { it[0] to it.getOrElse(1) { "" } }
            whole.startsWith(before) && whole.endsWith(after)
        }
        val scale = when (unit) {
            units.gib -> GIB
            units.mib -> MIB
            units.kib -> KIB
            else -> 1.0
        }
        val part = done.coerceIn(0, total).toDouble() / scale
        val fraction = if (scale == GIB || (scale == MIB && part < 100)) 1 else 0
        return ofTotal.format(number(part, fraction), whole)
    }

    /** What is left at [bytesPerSecond]: «0:56», «12:03», «1:02:09»; null if unknown. */
    fun left(remaining: Long, bytesPerSecond: Long): String? {
        if (bytesPerSecond <= 0 || remaining <= 0) return null
        val seconds = Math.ceil(remaining.toDouble() / bytesPerSecond).toLong()
        if (seconds >= 100 * 3600) return null
        val h = seconds / 3600
        val m = seconds % 3600 / 60
        val s = seconds % 60
        return if (h > 0) "%d:%02d:%02d".format(Locale.ROOT, h, m, s) else "%d:%02d".format(Locale.ROOT, m, s)
    }
}
