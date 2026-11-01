package io.github.motalim_ai.zajil.downloads

/**
 * The desktop's five built-in categories (`zajil-core` category.rs,
 * `CategoriesFile::built_in`), by the same extensions: with «A folder for
 * each type» (0.3.0), a finished file goes into its category's folder, named
 * in the app's language as the desktop names it (`locales/ar/zajil.ftl` and `locales/en/zajil.ftl`,
 * `category-*`). A file of no category stays in the save folder itself.
 */
enum class SaveCategory(val extensions: Set<String>) {
    Programs(setOf("exe", "msi", "msix", "msu", "dmg", "pkg", "deb", "rpm", "appimage", "apk", "flatpak", "iso", "img", "bin")),
    Video(
        setOf(
            "mp4", "mkv", "avi", "mov", "webm", "wmv", "flv", "m4v", "mpg", "mpeg", "3gp", "3g2", "rmvb", "rm", "ogv", "m2ts",
            "mts", "vob", "asf", "qt", "f4v",
        ),
    ),
    Music(setOf("mp3", "flac", "m4a", "aac", "ogg", "opus", "wav", "wma", "aif", "aiff", "ra")),
    Documents(setOf("pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "epub", "txt", "rtf", "csv")),
    Archives(
        setOf(
            "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "arj", "lzh", "cab",
            "r00", "r01", "r02", "r03", "r04", "r05", "r06", "r07", "r08", "r09",
            "r10", "r11", "r12", "r13", "r14", "r15", "r16", "r17", "r18", "r19",
        ),
    ),
    ;

    companion object {
        /** [name]'s category by its extension, in the desktop's order; null for none. */
        fun of(name: String): SaveCategory? {
            // A name and an extension: «README» and «.pdf» have no category.
            if (!name.contains('.') || name.substringBeforeLast('.').isEmpty()) return null
            val ext = name.substringAfterLast('.').lowercase()
            return entries.firstOrNull { ext in it.extensions }
        }
    }
}
