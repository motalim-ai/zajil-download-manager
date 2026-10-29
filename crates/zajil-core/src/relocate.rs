//! Moving a download that has already begun to another folder or name.
//!
//! «Download file info» starts the download in the background as soon as
//! the server answers (the owner's request after trying other download
//! managers): by the time the user picks the folder and the name, much of
//! the file is there. «Start download» then moves what is on disk to the
//! chosen place, and the download goes on from there.
//!
//! An unfinished download is its `.zajil-part` and its `.zajil-state`
//! (ADR-0004); the state file holds no path, so moving both under a new
//! final name is all a resume needs. A finished one is its file alone.
//! Within one volume this is a rename; to another drive the data is
//! copied, synced, and only then is the old copy removed.
//!
//! Blocking: call it from a blocking thread, never inside `async` code. The
//! download must not run meanwhile (the caller pauses it first).

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use crate::naming::{ReserveError, reserve_target};
use crate::sanitize::{ConfineError, confine, sanitize_file_name};
use crate::state::{DownloadState, StateError, final_path_for_state, part_path, state_path};

/// Why a download could not be moved. Nothing of it was lost: on an error,
/// the files stay where they were.
#[derive(Debug, thiserror::Error)]
pub enum RelocateError {
    #[error("{} is not a state file", .0.display())]
    NotAStateFile(PathBuf),
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Reserve(#[from] ReserveError),
    #[error(transparent)]
    Confine(#[from] ConfineError),
    #[error("could not move the download: {0}")]
    Io(#[from] io::Error),
}

/// Moves the unfinished download of `state_file` into `dir`, under `name`
/// (cleaned as any name; numbered if taken there). Returns its new state
/// file, which [`Engine::resume`](crate::Engine::resume) takes. Already
/// there under that name: nothing moves.
pub fn move_unfinished(
    state_file: &Path,
    dir: &Path,
    name: &str,
) -> Result<PathBuf, RelocateError> {
    move_unfinished_with(state_file, dir, name, false)
}

/// Moves the finished file `file` into `dir`, under `name` (cleaned, and
/// numbered if taken there). Returns where it is now.
pub fn move_finished(file: &Path, dir: &Path, name: &str) -> Result<PathBuf, RelocateError> {
    move_finished_with(file, dir, name, false)
}

/// Whether `path` is `name` in `dir` already (both as the engine makes
/// them: the folder resolved).
fn already_there(path: &Path, dir: &Path, name: &str) -> Result<bool, ConfineError> {
    Ok(confine(dir, name)? == path)
}

/// [`move_unfinished`]; `copy`: as between two drives, even on one (the
/// tests of the copy).
fn move_unfinished_with(
    state_file: &Path,
    dir: &Path,
    name: &str,
    copy: bool,
) -> Result<PathBuf, RelocateError> {
    let old_final = final_path_for_state(state_file)
        .ok_or_else(|| RelocateError::NotAStateFile(state_file.to_path_buf()))?;
    let name = sanitize_file_name(name);
    if already_there(&old_final, dir, &name)? {
        return Ok(state_file.to_path_buf());
    }
    // Checked and read before anything moves: a damaged state or a part
    // that is a link stays as it is.
    let mut state = DownloadState::load(state_file)?;
    let old_part = part_path(&old_final);
    let mut source = match zajil_platform::fs::open_regular_nofollow(&old_part, false) {
        Ok(file) => Some(file),
        // No data yet: the state alone moves.
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(err.into()),
    };
    let (new_final, reserved) = reserve_target(dir, &name)?;
    let new_part = part_path(&new_final);
    let new_state = state_path(&new_final);
    let undo = |reserved: Option<File>| {
        drop(reserved);
        let _ = zajil_platform::fs::remove_file_retrying(&new_state);
        let _ = zajil_platform::fs::remove_file_retrying(&new_part);
    };
    let mut reserved = Some(reserved);
    if let Some(from) = source.as_mut() {
        let renamed = !copy && {
            // The reserved file is replaced by the part (it is ours, and
            // empty); its handle closes first, for Windows.
            drop(reserved.take());
            std::fs::rename(&old_part, &new_part).is_ok()
        };
        if !renamed {
            let into = match reserved.take() {
                Some(file) => file,
                None => match zajil_platform::fs::open_regular_nofollow(&new_part, true) {
                    Ok(file) => file,
                    Err(err) => {
                        undo(None);
                        return Err(err.into());
                    }
                },
            };
            if let Err(err) = copy_synced(from, &into) {
                undo(Some(into));
                return Err(err.into());
            }
        }
    }
    drop(reserved);
    drop(source);
    if let Err(err) = state.save(&new_state) {
        // The part may already be at its new name: it goes back.
        if !old_part.exists() && new_part.exists() {
            let _ = std::fs::rename(&new_part, &old_part);
        }
        undo(None);
        return Err(err.into());
    }
    zajil_platform::fs::sync_dir(dir)?;
    // The old copy goes once the new one is whole.
    zajil_platform::fs::remove_file_retrying(state_file)?;
    zajil_platform::fs::remove_file_retrying(&old_part)?;
    Ok(new_state)
}

/// [`move_finished`]; `copy` as for [`move_unfinished_with`].
fn move_finished_with(
    file: &Path,
    dir: &Path,
    name: &str,
    copy: bool,
) -> Result<PathBuf, RelocateError> {
    let name = sanitize_file_name(name);
    if already_there(file, dir, &name)? {
        return Ok(file.to_path_buf());
    }
    let mut from = zajil_platform::fs::open_regular_nofollow(file, false)?;
    let (new_final, reserved) = reserve_target(dir, &name)?;
    let new_part = part_path(&new_final);
    let moved = !copy && zajil_platform::fs::rename_noreplace(file, &new_final).is_ok();
    if moved {
        drop(reserved);
        let _ = zajil_platform::fs::remove_file_retrying(&new_part);
        zajil_platform::fs::sync_dir(dir)?;
        return Ok(new_final);
    }
    let copied = copy_synced(&mut from, &reserved);
    drop(reserved);
    drop(from);
    if let Err(err) =
        copied.and_then(|()| zajil_platform::fs::rename_noreplace(&new_part, &new_final))
    {
        let _ = zajil_platform::fs::remove_file_retrying(&new_part);
        return Err(err.into());
    }
    zajil_platform::fs::sync_dir(dir)?;
    zajil_platform::fs::remove_file_retrying(file)?;
    Ok(new_final)
}

/// Copies all of `from` into `into` and syncs it.
fn copy_synced(from: &mut File, into: &File) -> io::Result<()> {
    let mut writer = into;
    io::copy(from, &mut writer)?;
    into.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Validator;

    /// An unfinished download in `dir`: its part (`data`) and its state.
    fn unfinished(dir: &Path, name: &str, data: &[u8]) -> PathBuf {
        let final_path = dir.join(name);
        std::fs::write(part_path(&final_path), data).unwrap();
        let mut state = DownloadState::new(
            "https://example.test/f".into(),
            "https://example.test/f".into(),
            Validator::default(),
            Some(data.len() as u64 * 2),
            true,
        );
        state.segments[0].done = data.len() as u64;
        let path = state_path(&final_path);
        state.save(&path).unwrap();
        path
    }

    /// A temporary folder, resolved as the engine resolves folders.
    fn tempdir() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = dunce::canonicalize(tmp.path()).unwrap();
        (tmp, path)
    }

    fn check_moved(old_state: &Path, new_state: &Path, data: &[u8]) {
        let old_final = final_path_for_state(old_state).unwrap();
        assert!(!old_state.exists() && !part_path(&old_final).exists());
        let new_final = final_path_for_state(new_state).unwrap();
        assert_eq!(std::fs::read(part_path(&new_final)).unwrap(), data);
        let state = DownloadState::load(new_state).unwrap();
        assert_eq!(state.bytes_done(), data.len() as u64);
        // The state keeps its token-safe permissions where it went.
        assert!(zajil_platform::fs::only_owner_can_access(new_state).unwrap());
    }

    #[test]
    fn an_unfinished_download_moves_to_another_folder_and_name() {
        for copy in [false, true] {
            let (_tmp, root) = tempdir();
            let (a, b) = (root.join("a"), root.join("b"));
            std::fs::create_dir_all(&a).unwrap();
            std::fs::create_dir_all(&b).unwrap();
            let old = unfinished(&a, "file.zip", b"half of it");
            let new = move_unfinished_with(&old, &b, "ملف جديد.zip", copy).unwrap();
            assert_eq!(new, b.join("ملف جديد.zip.zajil-state"));
            check_moved(&old, &new, b"half of it");
        }
    }

    #[test]
    fn a_taken_name_is_numbered_and_the_same_place_moves_nothing() {
        let (_tmp, root) = tempdir();
        let old = unfinished(&root, "a.bin", b"x");
        assert_eq!(move_unfinished(&old, &root, "a.bin").unwrap(), old);
        std::fs::write(root.join("b.bin"), b"someone else's").unwrap();
        let new = move_unfinished(&old, &root, "b.bin").unwrap();
        assert_eq!(new, root.join("b (1).bin.zajil-state"));
        check_moved(&old, &new, b"x");
        assert_eq!(
            std::fs::read(root.join("b.bin")).unwrap(),
            b"someone else's"
        );
    }

    #[test]
    fn a_name_cannot_leave_the_folder() {
        let (_tmp, root) = tempdir();
        let inside = root.join("in");
        std::fs::create_dir_all(&inside).unwrap();
        let old = unfinished(&inside, "a.bin", b"x");
        let new = move_unfinished(&old, &inside, "../../out.bin").unwrap();
        assert_eq!(new.parent(), Some(inside.as_path()));
        check_moved(&old, &new, b"x");
    }

    /// A part that is a link (symbolic or hard) to a file outside is
    /// refused, and the file outside is unchanged.
    #[test]
    fn a_linked_part_is_refused() {
        for hard in [false, true] {
            let tmp = tempfile::tempdir().unwrap();
            let (dir, to) = (tmp.path().join("d"), tmp.path().join("to"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::create_dir_all(&to).unwrap();
            let outside = tmp.path().join("outside");
            std::fs::write(&outside, b"keep me").unwrap();
            let old = unfinished(&dir, "a.bin", b"x");
            let part = part_path(&dir.join("a.bin"));
            std::fs::remove_file(&part).unwrap();
            if hard {
                std::fs::hard_link(&outside, &part).unwrap();
            } else {
                zajil_platform::fs::symlink_file(&outside, &part)
                    .expect("the test needs a symbolic link");
            }
            assert!(move_unfinished(&old, &to, "b.bin").is_err());
            assert_eq!(std::fs::read(&outside).unwrap(), b"keep me");
            assert!(old.exists());
            assert_eq!(std::fs::read_dir(&to).unwrap().count(), 0);
        }
    }

    #[test]
    fn a_finished_file_moves() {
        for copy in [false, true] {
            let (_tmp, root) = tempdir();
            let to = root.join("to");
            std::fs::create_dir_all(&to).unwrap();
            let file = root.join("done.txt");
            std::fs::write(&file, b"all of it").unwrap();
            let new = move_finished_with(&file, &to, "renamed.txt", copy).unwrap();
            assert_eq!(new, to.join("renamed.txt"));
            assert_eq!(std::fs::read(&new).unwrap(), b"all of it");
            assert!(!file.exists());
            assert!(!part_path(&new).exists());
        }
    }
}
