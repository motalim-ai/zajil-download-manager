//! File-system operations whose safe behaviour differs between systems.
//!
//! - [`sync_dir`]: make a rename inside a folder durable.
//! - [`replace_file`]: replace a file in one step (the state file).
//! - [`rename_noreplace`]: rename without ever overwriting (finishing a
//!   download).
//! - [`open_regular_nofollow`]: open a file only if it is a plain file, never
//!   through a symbolic link, junction or hard link.
//! - [`write_at`]: write at a byte offset through one shared handle (many
//!   connections writing one download).
//! - [`allocate`] and [`available_space`]: size a new download and check
//!   that it fits before any data arrives.
//! - [`create_private`] and [`only_owner_can_access`]: files that only
//!   their owner may read (the state file can hold a token from the URL).
//!
//! On Windows, renames retry briefly when another program (antivirus, search
//! indexer, OneDrive, backup tools) holds one of the files open, and pass
//! `MOVEFILE_WRITE_THROUGH`. Microsoft documents that this flag makes
//! `MoveFileExW` wait until the file is on disk only for moves done by copy
//! and delete (to another volume). For a rename within one volume, which is
//! our case, it is a request for write-through, not a documented guarantee
//! that the rename is durable when the call returns.
//!
//! The retries sleep the calling thread (about one second at most), so async
//! code must call these functions from a blocking task
//! (`tokio::task::spawn_blocking`).

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A file we refused to open because it is a link or not a plain file.
#[derive(Debug, thiserror::Error)]
#[error(
    "refusing to use {}: it is a symbolic link, a hard link or not a regular file",
    path.display()
)]
pub struct NotRegularFile {
    pub path: PathBuf,
}

/// Whether `err` came from [`open_regular_nofollow`] refusing a file.
pub fn is_not_regular_file(err: &io::Error) -> bool {
    err.get_ref()
        .is_some_and(|inner| inner.is::<NotRegularFile>())
}

fn not_regular(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        NotRegularFile {
            path: path.to_path_buf(),
        },
    )
}

/// Makes a rename or file creation inside `dir` durable.
///
/// On macOS and Linux the directory entry is only safe on disk after the
/// directory itself is synced (ADR-0004, safe-write rule 2). On Windows the
/// standard library cannot open a folder to sync it; the renames in this
/// module request write-through instead (see the module notes), so this
/// only checks that the folder exists.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    imp::sync_dir(dir)
}

/// Replaces `to` with `from` in one step. Both must be in the same folder.
pub fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    with_retry(|| imp::replace_file(from, to))
}

/// Renames `from` to `to`, failing with [`io::ErrorKind::AlreadyExists`] if
/// `to` exists. It never replaces an existing file that is there when the
/// call starts.
///
/// The rename itself refuses to replace (`renameat2(RENAME_NOREPLACE)` on
/// Linux, `renamex_np(RENAME_EXCL)` on macOS, `MoveFileExW` without
/// `MOVEFILE_REPLACE_EXISTING` on Windows), so even a file that appears at the
/// last moment is safe. Only where the file system refuses that call, a hard
/// link and then removing the old name is used instead, which is just as
/// safe.
///
/// **Last resort, with a documented race.** Some file systems refuse both:
/// exFAT on macOS (tested), and FAT or exFAT with older kernels. Only then,
/// the name is checked and a plain `rename` follows ([`check_then_rename`]).
/// A file that another program creates at `to` in the instant between the
/// check and the rename would be replaced. Zajil's own downloads cannot race
/// each other there: a download only renames to a name whose `.zajil-part`
/// it holds, or to a free name nobody else reserved.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    with_retry(|| imp::rename_noreplace(from, to))
}

/// Moves `path` aside to a new name in the same folder
/// (`.zajil-removed-<process>-<n>`, a dot-name: hidden on macOS and Linux,
/// not on Windows), for the caller to remove: its own name
/// is free at once, even while another program holds the file open.
///
/// Removing a file in place does not free its name on Windows while any
/// handle on it is open (an antivirus, the search indexer, or our own): the
/// file is only "delete pending" until the last handle closes, and creating
/// a file of that name meanwhile fails with `ERROR_ACCESS_DENIED` (#55). A
/// renamed file takes its pending deletion away under the hidden name.
///
/// The rename never replaces anything ([`rename_noreplace`]; only on file
/// systems that refuse a no-replace rename, its documented last resort
/// leaves an instant in which another process creating exactly this name
/// could lose it), and does not
/// retry, so it may be called where sleeping is not allowed (a `Drop` on an
/// async thread). It fails like the rename: `NotFound` when `path` does not
/// exist, and an error when a program holds the file without sharing its
/// deletion (then the file and its name stay as they were).
pub fn move_aside(path: &Path) -> io::Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    loop {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let aside = dir.join(format!(".zajil-removed-{}-{n}", std::process::id()));
        match imp::rename_noreplace(path, &aside) {
            // Left by an earlier process that had the same number.
            Err(err)
                if err.kind() == io::ErrorKind::AlreadyExists
                    && aside.symlink_metadata().is_ok() => {}
            result => return result.map(|()| aside),
        }
    }
}

/// Opens `path` for reading, and for writing when `write` is true, only if
/// it is a regular file with a single name (or none any more: a file
/// replaced or removed right after it was opened, which is not a link). A symbolic link, junction or
/// other reparse point, hard link, folder, device or pipe is refused with a
/// [`NotRegularFile`] error, checked on the opened handle itself so the file
/// cannot be swapped between the check and the use.
pub fn open_regular_nofollow(path: &Path, write: bool) -> io::Result<File> {
    imp::open_regular_nofollow(path, write)
}

/// Creates `path` for reading and writing, failing if anything already
/// exists there (like `create_new`), with access for the current user only,
/// from the moment it exists: mode `0600` on macOS and Linux, and on
/// Windows a protected access list (nothing inherited from the folder) with
/// one entry, full access for the current user. A rename in the same
/// folder keeps this.
///
/// For files that may hold a secret: the state file keeps a token that is
/// part of the URL (`?sig=…`), because resuming needs it (owner's decision
/// on #28).
pub fn create_private(path: &Path) -> io::Result<File> {
    imp::create_private(path)
}

/// Whether nobody but the current user may open `path`, read from the file
/// system itself: no permission bits for the group or others on macOS and
/// Linux; on Windows a protected access list whose only entries allow the
/// current user. For checks and tests of [`create_private`].
pub fn only_owner_can_access(path: &Path) -> io::Result<bool> {
    imp::only_owner_can_access(path)
}

/// Whether no other user can change `path` (a program a browser will be
/// told to start, or its folder): on macOS and Linux it is owned by the
/// current user or root, and not writable by its group or others. On
/// Windows its owner is the current user, SYSTEM, the Administrators or
/// TrustedInstaller, and its access list lets no one else write to it,
/// add to it, delete it or change its permissions or owner (review #84:
/// this was not checked at all). Read without following a link at `path`.
pub fn only_user_or_root_can_change(path: &Path) -> io::Result<bool> {
    imp::only_user_or_root_can_change(path)
}

/// Whether no other user can put something else in place of what `dir`
/// holds (a folder above a program Zajil runs): [`only_user_or_root_can_change`],
/// or on macOS and Linux a sticky folder of the user or root (like `/tmp`:
/// others may add, but never rename or remove what is not theirs). On
/// Windows a volume's root counts too: its usual access list lets users
/// add folders, not replace or remove those of others.
pub fn no_other_user_can_replace_in(dir: &Path) -> io::Result<bool> {
    if only_user_or_root_can_change(dir)? {
        return Ok(true);
    }
    imp::others_only_add_in(dir)
}

/// Makes `dir` a folder that no other user can change, creating it and
/// each missing parent as one: mode `0700` on macOS and Linux (whatever the
/// umask, and on macOS without an inherited access list); on Windows an
/// access list for the user alone, which what is created inside inherits.
/// An existing `dir` must already be one ([`is_private_dir`], never through
/// a link). For the video tools, which Zajil runs (ADR-0009), and the
/// folders of each run.
pub fn private_dir(dir: &Path) -> io::Result<()> {
    imp::create_private_dir(dir)?;
    if !is_private_dir(dir)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} is not a folder only this user can change (it must be a folder, not a link)",
                dir.display()
            ),
        ));
    }
    Ok(())
}

/// Whether `dir` is a folder (not a link) that only this user, or the
/// system, can change: [`only_user_or_root_can_change`], and on macOS no
/// access list entry (which could let others change it whatever its mode).
pub fn is_private_dir(dir: &Path) -> io::Result<bool> {
    let meta = std::fs::symlink_metadata(dir)?;
    Ok(meta.is_dir() && only_user_or_root_can_change(dir)? && !imp::dir_has_acl(dir)?)
}

/// Opens the program `path` to check it before it runs (ADR-0009): a
/// regular file, never through a link, that only this user or the system
/// can change, checked on the opened file. On Windows it stays locked
/// while the handle is open: nobody can write, delete or rename it, or
/// rename a folder above it, until it has started.
pub fn open_private_tool(path: &Path) -> io::Result<File> {
    imp::open_private_tool(path)
}

/// Lets the owner run `file` (mode `0700` on macOS and Linux; nothing to do
/// on Windows, where a `.exe` runs by its name). For a tool Zajil checked
/// before saving it (ADR-0009).
pub fn make_executable(file: &File) -> io::Result<()> {
    imp::make_executable(file)
}

/// Gives a finished download made by a tool under `umask 077` the mode of
/// any other download (`0644` on macOS and Linux; nothing to do on
/// Windows, where it took its folder's access list).
pub fn make_ordinary(file: &File) -> io::Result<()> {
    imp::make_ordinary(file)
}

/// Removes the file `path` (never what a link points to), again for about
/// a second while another program (an antivirus) holds it. A missing file
/// is fine.
pub fn remove_file_retrying(path: &Path) -> io::Result<()> {
    with_retry(|| match std::fs::remove_file(path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    })
}

/// Removes the folder `dir` with what is in it (a link is removed, not
/// followed), again for about a second while another program holds a file
/// in it. A missing folder is fine.
pub fn remove_dir_all_retrying(dir: &Path) -> io::Result<()> {
    with_retry(|| match std::fs::remove_dir_all(dir) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    })
}

/// Creates a symbolic link `link` pointing to the file `target`. Used by
/// tests that check symbolic links are refused. On Windows this needs
/// Developer Mode or administrator rights.
pub fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
    imp::symlink_file(target, link)
}

/// Creates a symbolic link `link` pointing to the folder `target`. Used by
/// tests that check links in place of folders are refused. On Windows this
/// needs Developer Mode or administrator rights.
pub fn symlink_dir(target: &Path, link: &Path) -> io::Result<()> {
    imp::symlink_dir(target, link)
}

/// For tests: opens `path` for reading the way an antivirus scanner or
/// indexer does. On Windows it does not allow deletion or renaming while the
/// returned file is open (no `FILE_SHARE_DELETE`), so renames over `path`
/// fail until it is dropped. On macOS and Linux it is a plain open, which
/// never blocks a rename.
pub fn open_like_a_scanner(path: &Path) -> io::Result<File> {
    imp::open_like_a_scanner(path)
}

/// Writes all of `buf` at byte `offset` of `file`.
///
/// Many connections write one download through the same handle, the one
/// that holds the lock (on Windows a lock belongs to a handle, so a second
/// handle could not write at all). On macOS and Linux this is `pwrite`,
/// which leaves the file position alone; on Windows it moves the position,
/// so a handle used with this function must only be written with it.
pub fn write_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    imp::write_at(file, buf, offset)
}

/// Bytes that this user may still write on the volume holding `dir` (an
/// existing folder).
pub fn available_space(dir: &Path) -> io::Result<u64> {
    imp::available_space(dir)
}

/// Gives the empty `file` the length `len`, for a download whose size is
/// known and whose pieces arrive in any order.
///
/// Nothing is reserved on any system (owner's decision, 2026-10-28): the
/// file is sparse, and its blocks are taken only as data arrives. A file
/// that took its whole size at once filled the disk before a byte came,
/// and a second download of the same link then failed at once.
///
/// - **macOS and Linux**: `set_len`. APFS, ext4 and btrfs keep the gap as
///   a hole, and a write far from the start fills only its own blocks
///   (`tests/sparse_volumes.rs`).
/// - **Windows**: the file is marked sparse, then `set_len`. On NTFS a plain
///   `set_len` would make every write far from the start fill the gap before
///   it with zeros first, writing a split download about twice.
///
/// So callers check [`available_space`] before starting (owner decision,
/// M2), and a disk that fills up later fails a write with the system's
/// error (`ENOSPC`, `ERROR_DISK_FULL`).
///
/// File systems that cannot have holes (FAT and exFAT; HFS+, not tested)
/// may write zeros up to `len`, and FAT32 refuses files of 4 GiB or more; both are
/// errors from the system, reported as they are. A `file` that is not empty
/// is refused.
pub fn allocate(file: &File, len: u64) -> io::Result<()> {
    if file.metadata()?.len() != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "allocate needs an empty file",
        ));
    }
    if len == 0 {
        return Ok(());
    }
    if i64::try_from(len).is_err() {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            format!("{len} bytes is larger than any file can be"),
        ));
    }
    imp::allocate(file, len)
}

/// Bytes the file at `path` takes on its volume. Less than its length for a
/// sparse file: tests use it to show that [`allocate`] reserves nothing.
pub fn size_on_disk(path: &Path) -> io::Result<u64> {
    imp::size_on_disk(path)
}

/// The last-resort rename of [`rename_noreplace`]: fails with
/// [`io::ErrorKind::AlreadyExists`] if anything (a file, folder or link) is
/// at `to`, and otherwise renames. **Not atomic**: something created at `to`
/// between the check and the rename is replaced. Only for file systems that
/// refuse both the no-replace rename and hard links.
pub fn check_then_rename(from: &Path, to: &Path) -> io::Result<()> {
    match to.symlink_metadata() {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", to.display()),
        )),
        Err(err) if err.kind() == io::ErrorKind::NotFound => std::fs::rename(from, to),
        Err(err) => Err(err),
    }
}

/// Retries `op` while it fails with an error another program can cause for
/// a moment (Windows sharing and access errors). About one second in total.
pub(crate) fn with_retry(mut op: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let mut delay = Duration::from_millis(10);
    for _ in 0..8 {
        match op() {
            Err(err) if imp::is_transient(&err) => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(200));
            }
            result => return result,
        }
    }
    op()
}

#[cfg(unix)]
#[path = "unix.rs"]
mod imp;

#[cfg(windows)]
pub(crate) use imp::{owner_only, runs_as_current_user};

#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "windows.rs"]
mod imp;

#[cfg(test)]
mod tests;

/// Windows only: a program that holds a file open without
/// `FILE_SHARE_DELETE` (as antivirus scanners do) makes a rename fail for a
/// moment; the retry must ride it out.
#[cfg(all(test, windows))]
mod windows_tests;
