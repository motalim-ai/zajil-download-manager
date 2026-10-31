//! The files on disk: the state file, the finished file's place, locks and
//! free space.

use std::fs::{File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::sanitize::{MAX_NAME_BYTES, confine, sanitize_file_name};
use crate::state::{DownloadState, StateError, final_path_for_state, part_path, state_path};

use super::Error;
use super::transfer::NewDownload;

/// Fails with [`Error::NotEnoughSpace`] unless `needed` bytes fit in the free
/// space of `dir`, counting `reusable` bytes already set aside there.
pub(super) async fn check_space(dir: &Path, needed: u64, reusable: u64) -> Result<(), Error> {
    let folder = dir.to_path_buf();
    let available = on_own_thread(move || zajil_platform::fs::available_space(&folder)).await??;
    if needed > available.saturating_add(reusable) {
        return Err(Error::NotEnoughSpace {
            dir: dir.to_path_buf(),
            needed,
            available,
        });
    }
    Ok(())
}

/// The download's final path from its state file's path, once the path is
/// one Zajil would make: the state file and everything next to it are
/// untrusted (they may come from another machine).
pub(super) fn checked_final_path(state_file: &Path) -> Result<PathBuf, Error> {
    // The state file and everything next to it are untrusted: they may
    // come from another machine. Work with an absolute path inside the
    // real folder, with a name that sanitizing leaves unchanged, and
    // never follow a link (`.zajil-state`, `.zajil-part` and the
    // temporary file are all opened without following links).
    let state_file = std::path::absolute(state_file)?;
    let final_path =
        final_path_for_state(&state_file).ok_or(Error::State(StateError::NotAStateFile))?;
    let name = final_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let sanitized = sanitize_file_name(name);
    if sanitized != name {
        let reason = if name.len() > MAX_NAME_BYTES {
            format!(
                "the name is {} bytes long, more than the {MAX_NAME_BYTES}-byte limit",
                name.len()
            )
        } else if sanitized == format!("_{name}") {
            // Windows reads the part before the first dot, without
            // trailing spaces, as the device name.
            let device = name.split('.').next().unwrap_or(name).trim_end();
            format!(
                "`{device}` is a device name Windows reserves, so Zajil never uses it for a file"
            )
        } else {
            format!("it is not a name Zajil would create (Zajil would write {sanitized:?})")
        };
        return Err(Error::UnsafeName {
            name: name.to_owned(),
            reason,
        });
    }
    let final_path = confine(parent_dir(&final_path), name)?;
    Ok(final_path)
}

pub(super) fn lock(file: &File, final_path: &Path) -> Result<(), Error> {
    match file.try_lock() {
        Ok(()) => Ok(()),
        Err(TryLockError::WouldBlock) => Err(Error::Locked(final_path.to_path_buf())),
        Err(TryLockError::Error(err)) => Err(err.into()),
    }
}

/// Saves the state file on a blocking thread: on Windows a save may wait up
/// to about a second for another program to release the file, and that wait
/// must not stop the async runtime. `new` is set for a new download that has
/// no data yet (see [`EmptyDownloadCleanup`]).
pub(crate) async fn save_state(
    state: &mut DownloadState,
    path: &Path,
    new: Option<Arc<NewDownload>>,
) -> Result<(), Error> {
    let mut copy = state.clone();
    let path = path.to_path_buf();
    *state = blocking(move || {
        let abandoned = || {
            new.as_ref()
                .is_some_and(|new| new.abandoned.load(Ordering::SeqCst))
        };
        if abandoned() {
            return Err(StateError::Io(io::Error::other(
                "the download was abandoned",
            )));
        }
        // If the download is abandoned while this runs, `new` is the last
        // owner when this closure ends, and its drop removes what was written.
        copy.save(&path).map(|()| copy)
    })
    .await?;
    Ok(())
}

/// Runs a quick system query (such as the free space) on a thread of its
/// own: never on an async thread, and never queued behind slow file work on
/// the blocking pool, so a new download does not wait there before it has
/// created any file.
async fn on_own_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> io::Result<T> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("zajil-query".into())
        .spawn(move || {
            let _ = sender.send(work());
        })?;
    receiver
        .await
        .map_err(|_| io::Error::other("the query thread stopped"))
}

/// Runs blocking file-system work off the async threads.
pub(crate) async fn blocking<T: Send + 'static, E: From<io::Error> + Send + 'static>(
    work: impl FnOnce() -> Result<T, E> + Send + 'static,
) -> Result<T, E> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| E::from(io::Error::other(err)))?
}

/// Renames the finished `part` to `wanted`, or to `name (1).ext` and so on
/// if that name is taken, never replacing an existing file. Another
/// download's part or state file also counts as taken.
pub(super) fn place_final(part: &Path, wanted: &Path) -> io::Result<PathBuf> {
    let name = wanted
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned();
    for candidate in crate::naming::candidates(&name) {
        let target = wanted.with_file_name(candidate);
        let busy = target != wanted
            && (part_path(&target).symlink_metadata().is_ok()
                || state_path(&target).symlink_metadata().is_ok());
        if busy {
            continue;
        }
        match zajil_platform::fs::rename_noreplace(part, &target) {
            Ok(()) => return Ok(target),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::other(format!("no free file name for {name:?}")))
}

/// The folder of `path`, or `.` for a bare file name.
pub(super) fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_the_finished_file_next_to_a_taken_name() {
        let dir = tempfile::tempdir().unwrap();
        let wanted = dir.path().join("a.zip");
        std::fs::write(&wanted, b"old").unwrap();
        std::fs::write(dir.path().join("a (1).zip"), b"old").unwrap();
        // `a (2).zip` is another download in progress.
        std::fs::write(dir.path().join("a (2).zip.zajil-part"), b"").unwrap();
        let part = part_path(&wanted);
        std::fs::write(&part, b"new").unwrap();
        let placed = place_final(&part, &wanted).unwrap();
        assert_eq!(placed, dir.path().join("a (3).zip"));
        assert_eq!(std::fs::read(&wanted).unwrap(), b"old");
        assert_eq!(std::fs::read(placed).unwrap(), b"new");
    }
}
