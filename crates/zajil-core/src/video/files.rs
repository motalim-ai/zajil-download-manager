//! The video's files: moved into place, the folder kept for a resume, and
//! the parts left.

use std::io;
use std::path::{Path, PathBuf};

use super::download::Finish;
use super::runs::random_hex;
use super::{VideoError, fast};

/// Moves a finished video `file` into `folder` as `<name>.<its extension>`,
/// numbered if that is taken, never over another file: the user changed
/// where it goes while it downloaded. Another disk: copied, then removed.
/// Blocking.
pub fn move_video(file: &Path, folder: &Path, name: &str) -> io::Result<PathBuf> {
    let finish = |dest: &Path| Finish {
        dest: dest.to_owned(),
        name: name.to_owned(),
        origin: String::new(),
        keep: None,
    };
    match finish(folder).place(file) {
        Err(err) if err.kind() == io::ErrorKind::CrossesDevices => {
            let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
            let copy = folder.join(format!(".{}.zajil-move.{ext}", random_hex()?));
            let mut to = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&copy)?;
            let copied = zajil_platform::fs::open_regular_nofollow(file, false)
                .and_then(|mut from| io::copy(&mut from, &mut to))
                .and_then(|_| to.sync_all());
            drop(to);
            if let Err(err) = copied {
                let _ = zajil_platform::fs::remove_file_retrying(&copy);
                return Err(err);
            }
            let placed = finish(folder).place(&copy);
            match &placed {
                Ok(_) => zajil_platform::fs::remove_file_retrying(file)?,
                Err(_) => {
                    let _ = zajil_platform::fs::remove_file_retrying(&copy);
                }
            }
            placed
        }
        placed => placed,
    }
}

/// The folder yt-dlp keeps in `dest` for the video whose key is `key`
/// ([`VideoRunner::download_keeping`]), for its next run to continue.
pub fn kept_folder(dest: &Path, key: &str) -> PathBuf {
    dest.join(format!(".zajil-video-keep-{key}"))
}

/// Removes a kept folder (its video finished, or was deleted): never what
/// a link points to. Blocking.
pub fn remove_kept(folder: &Path) {
    if std::fs::symlink_metadata(folder).is_ok_and(|m| m.is_dir()) {
        let _ = zajil_platform::fs::remove_dir_all_retrying(folder);
    }
}

/// A key of letters and digits, at most 32: a folder's name, never a path.
pub(super) fn is_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 32 && key.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The part and state files a stopped YouTube download named `name` left
/// in `dest` (Zajil's own way, `<name>.f<id>.<ext>`), to remove them when
/// the user cancels it. Blocking.
pub fn part_files(dest: &Path, name: &str) -> Vec<PathBuf> {
    fast::part_files(dest, name)
}

/// `name` without a video or sound extension at its end: the file gets
/// the one yt-dlp chose, and a name that came with one would end
/// `Title.mkv.mp4`.
pub(super) fn without_media_extension(name: &str) -> &str {
    const MEDIA: [&str; 15] = [
        "3gp", "aac", "avi", "flac", "flv", "m4a", "m4v", "mkv", "mov", "mp3", "mp4", "ogg",
        "opus", "wav", "webm",
    ];
    match name.trim_end().rsplit_once('.') {
        Some((base, ext))
            if !base.trim().is_empty() && MEDIA.iter().any(|m| ext.eq_ignore_ascii_case(m)) =>
        {
            base
        }
        _ => name,
    }
}

pub(super) fn checked(url: &str) -> Result<reqwest::Url, VideoError> {
    crate::queue::check_url(url).map_err(|e| VideoError::Refused(e.to_string()))?;
    let parsed = reqwest::Url::parse(url).map_err(|e| VideoError::Refused(e.to_string()))?;
    // check_url accepts http and https only, so the URL never starts with
    // '-'; checked again here because the argument list relies on it.
    if !matches!(parsed.scheme(), "http" | "https") || url.starts_with('-') {
        return Err(VideoError::Refused("only http and https links".into()));
    }
    Ok(parsed)
}

/// Checks the path yt-dlp printed: a file directly inside the run's
/// output folder, not a link. Returns it.
pub(super) fn place(out: &Path, printed: &Path) -> io::Result<PathBuf> {
    let name = printed
        .file_name()
        .ok_or_else(|| io::Error::other("yt-dlp printed no file name"))?;
    let inside = out.join(name);
    let same = std::fs::canonicalize(printed)
        .ok()
        .zip(std::fs::canonicalize(&inside).ok())
        .is_some_and(|(a, b)| a == b);
    if !same {
        return Err(io::Error::other(
            "yt-dlp's file is not in its output folder",
        ));
    }
    drop(zajil_platform::fs::open_regular_nofollow(&inside, false)?);
    Ok(inside)
}
