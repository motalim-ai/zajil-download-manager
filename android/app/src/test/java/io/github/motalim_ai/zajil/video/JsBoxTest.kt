package io.github.motalim_ai.zajil.video

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

class JsBoxTest {
    @Test
    fun output_is_read_up_to_its_cap_and_the_rest_is_dropped() {
        val big = ByteArray(300_000) { (it % 251).toByte() }
        val read = JsBox.readCapped(ByteArrayInputStream(big), max = 100_000)
        assertEquals(100_000, read.size)
        assertArrayEquals(big.copyOf(100_000), read)
        assertArrayEquals(big, JsBox.readCapped(ByteArrayInputStream(big)))
    }

    @Test
    fun a_script_is_copied_up_to_its_cap() {
        val big = ByteArray(200_000) { 7 }
        val out = ByteArrayOutputStream()
        JsBox.copyCapped(ByteArrayInputStream(big), out, max = 65_000)
        assertEquals(65_000, out.size())
    }

    @Test
    fun the_bootstrap_takes_std_and_os_away_before_the_script_runs() {
        val boot = JsBoxService.BOOTSTRAP
        val read = boot.indexOf("std.in.readAsString()")
        val run = boot.indexOf("(0,eval)(t)")
        for (name in listOf("std", "os", "bjson")) {
            val gone = boot.indexOf("delete globalThis.$name")
            assert(gone in (read + 1) until run) { name }
        }
    }
}
