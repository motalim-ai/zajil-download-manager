package io.github.motalim_ai.zajil.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class KindTest {
    @Test
    fun kindsByExtensionInAnyCase() {
        assertEquals(Kind.Video, Kind.ofName("الحلقة 12.MP4"))
        assertEquals(Kind.Music, Kind.ofName("محاضرة.m4a"))
        assertEquals(Kind.Documents, Kind.ofName("تقرير-الربع-الثالث.pdf"))
        assertEquals(Kind.Programs, Kind.ofName("zajil-android-0.2.0.apk"))
        assertEquals(Kind.Archives, Kind.ofName("صور-الرحلة.zip"))
        assertEquals(Kind.Images, Kind.ofName("a.webp"))
    }

    @Test
    fun noExtensionIsAFile() {
        assertEquals(Kind.Other, Kind.ofName("download"))
        assertEquals(Kind.Other, Kind.ofName("archive.zip.part2"))
    }
}
