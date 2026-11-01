package io.github.motalim_ai.zajil.ui

import io.github.motalim_ai.zajil.downloads.Download
import io.github.motalim_ai.zajil.downloads.Status
import io.github.motalim_ai.zajil.video.Container

/** A file's kind, by its name's extension, as the desktop's categories group them. */
enum class Kind(val icon: ColorIcon) {
    Video(ColorIcon.Video),
    Music(ColorIcon.Music),
    Images(ColorIcon.Images),
    Documents(ColorIcon.Documents),
    Programs(ColorIcon.Programs),
    Archives(ColorIcon.Archives),
    Other(ColorIcon.File),
    ;

    companion object {
        private val byExtension: Map<String, Kind> = buildMap {
            listOf("mp4", "mkv", "webm", "avi", "mov", "m4v", "3gp", "flv", "wmv", "ts").forEach { put(it, Video) }
            listOf("mp3", "m4a", "aac", "ogg", "oga", "opus", "flac", "wav", "wma").forEach { put(it, Music) }
            listOf("jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "heic", "avif").forEach { put(it, Images) }
            listOf("pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "rtf", "odt", "ods", "odp", "epub", "csv")
                .forEach { put(it, Documents) }
            listOf("apk", "apks", "xapk", "exe", "msi", "dmg", "deb", "rpm", "appimage", "pkg").forEach { put(it, Programs) }
            listOf("zip", "rar", "7z", "tar", "gz", "bz2", "xz", "tgz", "zst", "iso").forEach { put(it, Archives) }
        }

        fun ofName(name: String): Kind {
            val ext = name.substringAfterLast('.', "").lowercase()
            return byExtension[ext] ?: Other
        }

        fun of(download: Download): Kind = when {
            download.video?.container == Container.M4a -> Music
            download.video != null -> Video
            else -> ofName(download.name)
        }
    }
}

/** The home screen's filters. */
enum class Filter {
    All, Active, Done, Video, Music, Images, Documents, Programs, Archives, Other;

    fun matches(download: Download): Boolean = when (this) {
        All -> true
        Active -> download.status in setOf(Status.Running, Status.Queued, Status.Merging, Status.Publishing)
        Done -> download.status == Status.Done
        Video -> Kind.of(download) == Kind.Video
        Music -> Kind.of(download) == Kind.Music
        Images -> Kind.of(download) == Kind.Images
        Documents -> Kind.of(download) == Kind.Documents
        Programs -> Kind.of(download) == Kind.Programs
        Archives -> Kind.of(download) == Kind.Archives
        Other -> Kind.of(download) == Kind.Other
    }
}
