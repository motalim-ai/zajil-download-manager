package io.github.motalim_ai.zajil.downloads

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DownloadJsonTest {
    @Test
    fun a_complete_download_not_saved_stays_so_after_a_restart() {
        val unsaved = Download(id = 7, url = "https://example.com/a.zip", status = Status.Failed, file = "/data/a.zip", unsaved = true)
        val read = Download.fromJson(JSONObject(unsaved.toJson().toString()))
        assertTrue(read.unsaved)
        assertEquals(unsaved, read)
    }

    @Test
    fun a_list_from_before_the_field_reads_as_saved() {
        val old = JSONObject().put("id", 1).put("url", "https://example.com/b.zip").put("status", "Done")
        assertFalse(Download.fromJson(old).unsaved)
    }
}
