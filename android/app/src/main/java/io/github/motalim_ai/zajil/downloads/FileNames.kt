package io.github.motalim_ai.zajil.downloads

/**
 * A file name as the user reads it, in an Arabic or an English screen: one
 * bidi-isolated unit (Unicode's isolates, UAX #9), so that what is around
 * it never moves its parts.
 *
 * A name that starts with Latin letters («Me at the zoo.mp4») reads left to
 * right as a whole. A name that starts with Arabic reads right to left, and
 * what follows its last Arabic letter («12.mp4», «.pdf», «(1).m4a») stays
 * together, left to right: «الحلقة 12.mp4» is never shown as «mp4.12 الحلقة».
 * Only for showing: the name itself, and what is saved, never has the marks.
 */
object FileNames {
    const val LRI = '⁦'
    const val RLI = '⁧'
    const val PDI = '⁩'

    /** [name] with its isolates. */
    fun display(name: String): String {
        val out = StringBuilder(name.length + 4)
        var at = 0
        for ((index, marks) in marks(name)) {
            out.append(name, at, index).append(marks)
            at = index
        }
        return out.append(name, at, name.length).toString()
    }

    /**
     * Where the isolates go in [name]: each index with the marks inserted
     * before the character there (at the length: after the last one), in
     * order. For a text field's visual transformation as well as [display].
     */
    fun marks(name: String): List<Pair<Int, String>> {
        if (name.isEmpty()) return emptyList()
        if (!rightToLeftFirst(name)) return listOf(0 to "$LRI", name.length to "$PDI")
        val lastRtl = name.indexOfLast(::isRtl)
        // Spaces between the Arabic and the tail stay with the Arabic.
        var tail = lastRtl + 1
        while (tail < name.length && name[tail].isWhitespace()) tail++
        return if (tail >= name.length) {
            listOf(0 to "$RLI", name.length to "$PDI")
        } else {
            listOf(0 to "$RLI", tail to "$LRI", name.length to "$PDI$PDI")
        }
    }

    /** Whether the first letter with a direction is right to left (Arabic, Hebrew…). */
    private fun rightToLeftFirst(name: String): Boolean {
        for (c in name) {
            if (isRtl(c)) return true
            if (Character.getDirectionality(c) == Character.DIRECTIONALITY_LEFT_TO_RIGHT) return false
        }
        return false
    }

    private fun isRtl(c: Char): Boolean = when (Character.getDirectionality(c)) {
        Character.DIRECTIONALITY_RIGHT_TO_LEFT, Character.DIRECTIONALITY_RIGHT_TO_LEFT_ARABIC -> true
        else -> false
    }
}
