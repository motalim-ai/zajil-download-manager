package io.github.motalim_ai.zajil.ui

import io.github.motalim_ai.zajil.downloads.Download
import io.github.motalim_ai.zajil.downloads.Status
import io.github.motalim_ai.zajil.video.Container

/** A file's kind, by its name's extension, as the desktop's categories group them. */
enum class Kind(val icon: LineIcon) {
    Video(LineIcon.Video),
    Music(LineIcon.Music),
    Images(LineIcon.Image),
    Documents(LineIcon.File),
    Programs(LineIcon.App),
    Archives(LineIcon.Archive),
    Other(LineIcon.File),
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
            // A playlist's video not read yet: the sound alone, or a video.
            download.plan != null -> if (download.plan.height == null) Music else Video
            else -> ofName(download.name)
        }
    }
}

/** Where a download is, as the D5 design groups it: its row's colours and its filter chip. */
enum class Stage {
    Active, Done, Waiting, Failed;

    companion object {
        fun of(status: Status): Stage = when (status) {
            Status.Running, Status.Merging, Status.Publishing -> Active
            Status.Done -> Done
            Status.Failed -> Failed
            Status.Queued, Status.Paused, Status.Interrupted, Status.NeedsLocalNetwork -> Waiting
        }
    }
}

/** The downloads' filters: by stage (D5), then by kind when there is one of it. */
enum class Filter {
    All, Active, Done, Waiting, Failed, Playlists, Video, Music, Images, Documents, Programs, Archives, Other;

    fun matches(download: Download): Boolean = when (this) {
        All -> true
        Active -> Stage.of(download.status) == Stage.Active
        Done -> Stage.of(download.status) == Stage.Done
        Waiting -> Stage.of(download.status) == Stage.Waiting
        Failed -> Stage.of(download.status) == Stage.Failed
        // A playlist's videos go to its own folder (0.3.0).
        Playlists -> download.folder != null || download.plan != null
        Video -> Kind.of(download) == Kind.Video
        Music -> Kind.of(download) == Kind.Music
        Images -> Kind.of(download) == Kind.Images
        Documents -> Kind.of(download) == Kind.Documents
        Programs -> Kind.of(download) == Kind.Programs
        Archives -> Kind.of(download) == Kind.Archives
        Other -> Kind.of(download) == Kind.Other
    }

    companion object {
        /** The home's «النشاط» card: the stages but «failed». */
        val HOME = listOf(All, Active, Done, Waiting)
        val STAGES = listOf(All, Active, Done, Waiting, Failed)
        val KINDS = listOf(Video, Music, Images, Documents, Programs, Archives, Other)
    }
}
