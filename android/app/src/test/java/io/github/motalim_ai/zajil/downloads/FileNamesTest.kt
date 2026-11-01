package io.github.motalim_ai.zajil.downloads

import io.github.motalim_ai.zajil.downloads.FileNames.LRI
import io.github.motalim_ai.zajil.downloads.FileNames.PDI
import io.github.motalim_ai.zajil.downloads.FileNames.RLI
import org.junit.Assert.assertEquals
import org.junit.Test

class FileNamesTest {
    @Test
    fun anArabicNameKeepsItsNumberAndExtensionTogether() {
        assertEquals("${RLI}الحلقة ${LRI}12.mp4$PDI$PDI", FileNames.display("الحلقة 12.mp4"))
        assertEquals("${RLI}محاضرة الفيزياء ${LRI}3 (1).m4a$PDI$PDI", FileNames.display("محاضرة الفيزياء 3 (1).m4a"))
    }

    @Test
    fun theExtensionOfAnArabicNameIsOneLeftToRightUnit() {
        assertEquals("${RLI}كيف تعمل الأقمار الصناعية${LRI}.mp4$PDI$PDI", FileNames.display("كيف تعمل الأقمار الصناعية.mp4"))
        assertEquals("${RLI}تقرير-الربع-الثالث${LRI}.pdf$PDI$PDI", FileNames.display("تقرير-الربع-الثالث.pdf"))
    }

    @Test
    fun aLatinNameReadsLeftToRightEvenWithArabicInIt() {
        assertEquals("${LRI}Me at the zoo.mp4$PDI", FileNames.display("Me at the zoo.mp4"))
        assertEquals("${LRI}Report تقرير.pdf$PDI", FileNames.display("Report تقرير.pdf"))
        // Digits and signs first: the first letter decides.
        assertEquals("${RLI}2026 تقرير${LRI}.pdf$PDI$PDI", FileNames.display("2026 تقرير.pdf"))
    }

    @Test
    fun anArabicNameWithNothingAfterIt() {
        assertEquals("${RLI}ملف بلا امتداد$PDI", FileNames.display("ملف بلا امتداد"))
    }

    @Test
    fun theMarksAreOnlyAdded() {
        listOf("", "a", "الحلقة 12.mp4", "zajil-android-0.2.0.apk", "صور.zip").forEach { name ->
            val shown = FileNames.display(name)
            assertEquals(name, shown.filterNot { it == LRI || it == RLI || it == PDI })
        }
        assertEquals("", FileNames.display(""))
    }

    @Test
    fun aCdnNameGetsItsMediaExtension() {
        assertEquals("3weU6RoMs48WpDiakllyR.1034.IRZXSOY.mp4", FileNames.withType("3weU6RoMs48WpDiakllyR.1034.IRZXSOY", "video/mp4"))
        assertEquals("clip.mp4", FileNames.withType("clip.mp4", "video/mp4"))
        assertEquals("clip.m4v", FileNames.withType("clip.m4v", "video/mp4; codecs=avc1"))
        assertEquals("report.pdf", FileNames.withType("report.pdf", "application/pdf"))
        assertEquals("song", FileNames.withType("song", null))
        assertEquals("voice.mp3", FileNames.withType("voice", "audio/mpeg"))
    }
}
