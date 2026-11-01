package io.github.motalim_ai.zajil.browser

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ShortcutPresetsTest {
    private val presets = listOf(
        Preset("YouTube", "https://m.youtube.com/", since = 1),
        Preset("TikTok", "https://www.tiktok.com/", since = 1),
        Preset("Snapchat", "https://www.snapchat.com/spotlight", since = 2),
    )

    @Test
    fun a_changed_home_loses_vimeo_and_soundcloud_and_gains_snapchat_once() {
        val stored = listOf(
            Shortcut("YouTube", "https://m.youtube.com/"),
            Shortcut("SoundCloud", "https://m.soundcloud.com/"),
            Shortcut("Vimeo", "https://vimeo.com/"),
            Shortcut("My site", "https://example.com/"),
        )
        val now = ShortcutPresets.migrate(stored, from = 1, presets)
        assertEquals(
            listOf("https://m.youtube.com/", "https://example.com/", "https://www.snapchat.com/spotlight"),
            now.map { it.url },
        )
        // Already followed: nothing changes, even a preset the user removed stays removed.
        assertEquals(stored.take(1), ShortcutPresets.migrate(stored.take(1), from = 2, presets))
    }

    @Test
    fun a_vimeo_the_user_added_stays() {
        // The add sheet keeps what the user typed: not the app's exact address.
        val mine = listOf(Shortcut("Vimeo", "https://vimeo.com"), Shortcut("SC", "https://soundcloud.com/me"))
        assertEquals(mine + Shortcut("Snapchat", "https://www.snapchat.com/spotlight"), ShortcutPresets.migrate(mine, from = 1, presets))
        assertEquals(setOf("https://vimeo.com/", "https://m.soundcloud.com/"), ShortcutPresets.droppedSince(1))
    }

    @Test
    fun the_owners_channel_comes_once_to_a_changed_home() {
        val channel = Preset("Motalim", ShortcutPresets.MOTALIM_CHANNEL, since = 3)
        val stored = listOf(Shortcut("YouTube", "https://m.youtube.com/"), Shortcut("Snapchat", "https://www.snapchat.com/spotlight"))
        assertEquals(stored + Shortcut("Motalim", "https://www.youtube.com/@motalim-ai"), ShortcutPresets.migrate(stored, from = 2, presets + channel))
        // Removed by the user after that: not brought back.
        assertEquals(stored, ShortcutPresets.migrate(stored, from = 3, presets + channel))
        // An ASCII «@», as YouTube's own links have it.
        assertTrue(ShortcutPresets.MOTALIM_CHANNEL.all { it.code < 128 })
    }
}
