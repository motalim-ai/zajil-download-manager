package io.github.motalim_ai.zajil.downloads

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class SaveCategoryTest {
    @Test
    fun a_file_goes_to_the_desktops_category_by_its_extension() {
        assertEquals(SaveCategory.Video, SaveCategory.of("فيلم.MP4"))
        assertEquals(SaveCategory.Music, SaveCategory.of("song.m4a"))
        assertEquals(SaveCategory.Documents, SaveCategory.of("كتاب.pdf"))
        assertEquals(SaveCategory.Programs, SaveCategory.of("zajil.apk"))
        assertEquals(SaveCategory.Archives, SaveCategory.of("part.r07"))
        // The desktop puts «bin» under programs, before anything else.
        assertEquals(SaveCategory.Programs, SaveCategory.of("file.bin"))
        assertNull(SaveCategory.of("photo.jpg"))
        assertNull(SaveCategory.of("README"))
        assertNull(SaveCategory.of(".pdf"))
    }
}
