// Which of Zajil's coloured icons (`components/ColorIcon.svelte`) a file
// gets: by its category, or by its name's extension where none is known.

/** The icon of category `id` (`null`: none). */
export function typeIcon(id: string | null): string {
  const icons: Record<string, string> = {
    programs: "t-programs",
    video: "t-video",
    music: "t-music",
    documents: "t-documents",
    archives: "t-archives",
    images: "t-images",
  };
  return icons[id ?? ""] ?? "t-file";
}

const KINDS: [string, string[]][] = [
  ["t-archives", ["zip", "rar", "7z", "gz", "tar", "xz", "bz2", "tgz"]],
  ["t-documents", ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "rtf", "odt", "epub", "csv"]],
  ["t-music", ["mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", "wma"]],
  ["t-video", ["mp4", "mkv", "webm", "avi", "mov", "wmv", "flv", "m4v", "ts"]],
  ["t-images", ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "heic", "avif", "tif", "tiff"]],
  ["t-programs", ["exe", "msi", "dmg", "pkg", "deb", "rpm", "appimage", "apk"]],
];

/** The icon of a file by its name's extension, where no category is known
 * (the «Download complete» window). */
export function nameIcon(name: string): string {
  const ext = /\.([^.]+)$/.exec(name)?.[1]?.toLowerCase() ?? "";
  return KINDS.find(([, list]) => list.includes(ext))?.[0] ?? "t-file";
}
