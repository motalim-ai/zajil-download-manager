//! File-system operations whose safe behaviour differs between systems.
//!
//! - [`sync_dir`]: make a rename inside a folder durable.
//! - [`replace_file`]: replace a file in one step (the state file).
//! - [`rename_noreplace`]: rename without ever overwriting (finishing a
//!   download).
//! - [`open_regular_nofollow`]: open a file only if it is a plain file, never
//!   through a symbolic link, junction or hard link.
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
/// `to` exists. It never replaces an existing file, even if one appears at
/// the last moment.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    with_retry(|| imp::rename_noreplace(from, to))
}

/// Opens `path` for reading, and for writing when `write` is true, only if
/// it is a regular file with a single name. A symbolic link, junction or
/// other reparse point, hard link, folder, device or pipe is refused with a
/// [`NotRegularFile`] error, checked on the opened handle itself so the file
/// cannot be swapped between the check and the use.
pub fn open_regular_nofollow(path: &Path, write: bool) -> io::Result<File> {
    imp::open_regular_nofollow(path, write)
}

/// Creates a symbolic link `link` pointing to the file `target`. Used by
/// tests that check symbolic links are refused. On Windows this needs
/// Developer Mode or administrator rights.
pub fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
    imp::symlink_file(target, link)
}

/// For tests: opens `path` for reading the way an antivirus scanner or
/// indexer does. On Windows it does not allow deletion or renaming while the
/// returned file is open (no `FILE_SHARE_DELETE`), so renames over `path`
/// fail until it is dropped. On macOS and Linux it is a plain open, which
/// never blocks a rename.
pub fn open_like_a_scanner(path: &Path) -> io::Result<File> {
    imp::open_like_a_scanner(path)
}

/// Retries `op` while it fails with an error another program can cause for
/// a moment (Windows sharing and access errors). About one second in total.
fn with_retry(mut op: impl FnMut() -> io::Result<()>) -> io::Result<()> {
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
mod imp {
    use std::ffi::CString;
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::Path;

    use super::not_regular;

    pub fn sync_dir(dir: &Path) -> io::Result<()> {
        File::open(dir)?.sync_all()
    }

    pub fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
        // POSIX rename replaces atomically and never follows a link at `to`.
        std::fs::rename(from, to)
    }

    pub fn is_transient(_err: &io::Error) -> bool {
        false
    }

    pub fn open_regular_nofollow(path: &Path, write: bool) -> io::Result<File> {
        // O_NOFOLLOW refuses a symbolic link; O_NONBLOCK keeps a planted
        // FIFO from blocking the open.
        let file = OpenOptions::new()
            .read(true)
            .write(write)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|err| match err.raw_os_error() {
                Some(libc::ELOOP) | Some(libc::EMLINK) => not_regular(path),
                _ => err,
            })?;
        let meta = file.metadata()?;
        if !meta.file_type().is_file() || meta.nlink() != 1 {
            return Err(not_regular(path));
        }
        Ok(file)
    }

    pub fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    pub fn open_like_a_scanner(path: &Path) -> io::Result<File> {
        File::open(path)
    }

    #[allow(dead_code)]
    fn c_path(path: &Path) -> io::Result<CString> {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
    }

    #[cfg(target_os = "linux")]
    #[allow(unsafe_code)]
    pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
        let (from_c, to_c) = (c_path(from)?, c_path(to)?);
        // SAFETY: both pointers are valid NUL-terminated strings that live
        // until the call returns; AT_FDCWD resolves them like `rename`.
        let rc = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from_c.as_ptr(),
                libc::AT_FDCWD,
                to_c.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if rc == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            // The file system or kernel does not support the flag.
            Some(libc::EINVAL) | Some(libc::ENOSYS) => link_then_unlink(from, to),
            _ => Err(err),
        }
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
        let (from_c, to_c) = (c_path(from)?, c_path(to)?);
        // SAFETY: both pointers are valid NUL-terminated strings that live
        // until the call returns.
        let rc = unsafe { libc::renamex_np(from_c.as_ptr(), to_c.as_ptr(), libc::RENAME_EXCL) };
        if rc == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::ENOTSUP) | Some(libc::EINVAL) => link_then_unlink(from, to),
            _ => Err(err),
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
        link_then_unlink(from, to)
    }

    /// `link` fails if `to` exists, so it is a no-replace rename on file
    /// systems with hard links.
    fn link_then_unlink(from: &Path, to: &Path) -> io::Result<()> {
        std::fs::hard_link(from, to)?;
        std::fs::remove_file(from)
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION, HANDLE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_READ, GetFileInformationByHandle, MOVEFILE_REPLACE_EXISTING,
        MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    use super::not_regular;

    pub fn sync_dir(dir: &Path) -> io::Result<()> {
        std::fs::metadata(dir).map(|_| ())
    }

    pub fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
        move_file(from, to, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
    }

    pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
        // Without MOVEFILE_REPLACE_EXISTING the call fails if `to` exists;
        // the standard library maps that error to AlreadyExists.
        move_file(from, to, MOVEFILE_WRITE_THROUGH)
    }

    pub fn is_transient(err: &io::Error) -> bool {
        matches!(
            err.raw_os_error().map(|code| code as u32),
            Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
        )
    }

    pub fn open_regular_nofollow(path: &Path, write: bool) -> io::Result<File> {
        // Open the link or junction itself rather than its target.
        let file = OpenOptions::new()
            .read(true)
            .write(write)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let meta = file.metadata()?;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || !meta.file_type().is_file()
            || link_count(&file)? != 1
        {
            return Err(not_regular(path));
        }
        Ok(file)
    }

    pub fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    pub fn open_like_a_scanner(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)
    }

    fn link_count(file: &File) -> io::Result<u32> {
        // SAFETY: an all-zero BY_HANDLE_FILE_INFORMATION is a valid value.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: the handle is open for the lifetime of `file`, and `info`
        // is a valid, writable structure of the expected type.
        let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(info.nNumberOfLinks)
    }

    fn move_file(from: &Path, to: &Path, flags: u32) -> io::Result<()> {
        let (from_w, to_w) = (wide(from), wide(to));
        // SAFETY: both buffers are NUL-terminated UTF-16 strings that live
        // until the call returns.
        let ok = unsafe { MoveFileExW(from_w.as_ptr(), to_w.as_ptr(), flags) };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// NUL-terminated UTF-16, with the `\\?\` prefix for long absolute paths
    /// (the plain form is limited to 260 characters).
    fn wide(path: &Path) -> Vec<u16> {
        let text = path.as_os_str();
        let long =
            text.len() >= 248 && path.is_absolute() && !text.to_string_lossy().starts_with(r"\\");
        let prefix: &[u16] = if long {
            &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16]
        } else {
            &[]
        };
        prefix
            .iter()
            .copied()
            .chain(text.encode_wide())
            .chain(Some(0))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn syncs_an_existing_folder() {
        sync_dir(&std::env::temp_dir()).unwrap();
    }

    #[test]
    fn reports_a_missing_folder() {
        let missing = std::env::temp_dir().join("zajil-no-such-folder-for-sync");
        assert!(sync_dir(&missing).is_err());
    }

    #[test]
    fn replace_file_replaces() {
        let dir = tempdir();
        let (a, b) = (dir.join("a"), dir.join("b"));
        fs::write(&a, b"new").unwrap();
        fs::write(&b, b"old").unwrap();
        replace_file(&a, &b).unwrap();
        assert_eq!(fs::read(&b).unwrap(), b"new");
        assert!(!a.exists());
    }

    #[test]
    fn rename_noreplace_never_overwrites() {
        let dir = tempdir();
        let (a, b) = (dir.join("a"), dir.join("b"));
        fs::write(&a, b"new").unwrap();
        fs::write(&b, b"old").unwrap();
        let err = rename_noreplace(&a, &b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&b).unwrap(), b"old");
        assert_eq!(fs::read(&a).unwrap(), b"new");

        let c = dir.join("c");
        rename_noreplace(&a, &c).unwrap();
        assert_eq!(fs::read(&c).unwrap(), b"new");
        assert!(!a.exists());
    }

    #[test]
    fn opens_a_regular_file() {
        let dir = tempdir();
        let path = dir.join("f");
        fs::write(&path, b"x").unwrap();
        open_regular_nofollow(&path, true).unwrap();
        open_regular_nofollow(&path, false).unwrap();
    }

    #[test]
    fn refuses_a_symbolic_link_and_leaves_its_target_alone() {
        let dir = tempdir();
        let target = dir.join("victim");
        fs::write(&target, b"precious").unwrap();
        let link = dir.join("link");
        if !symlinks_available(&target, &link) {
            return;
        }
        let err = open_regular_nofollow(&link, true).unwrap_err();
        assert!(is_not_regular_file(&err), "{err}");
        assert_eq!(fs::read(&target).unwrap(), b"precious");
    }

    #[test]
    fn refuses_a_hard_link() {
        let dir = tempdir();
        let target = dir.join("victim");
        fs::write(&target, b"precious").unwrap();
        let link = dir.join("hard");
        fs::hard_link(&target, &link).unwrap();
        let err = open_regular_nofollow(&link, true).unwrap_err();
        assert!(is_not_regular_file(&err), "{err}");
    }

    #[test]
    fn refuses_a_folder() {
        let dir = tempdir();
        assert!(open_regular_nofollow(&dir, false).is_err());
    }

    /// Creates a symbolic link, or reports that this machine cannot. On CI
    /// (where `CI` is set) that is a failure, so the test cannot silently
    /// stop testing anything.
    fn symlinks_available(target: &Path, link: &Path) -> bool {
        match symlink_file(target, link) {
            Ok(()) => true,
            Err(err) if std::env::var_os("CI").is_none() => {
                eprintln!("skipping: cannot create symbolic links here: {err}");
                false
            }
            Err(err) => panic!("CI must be able to create symbolic links: {err}"),
        }
    }

    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "zajil-platform-fs-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}

/// Windows only: a program that holds a file open without
/// `FILE_SHARE_DELETE` (as antivirus scanners do) makes a rename fail for a
/// moment; the retry must ride it out.
#[cfg(all(test, windows))]
mod windows_tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn replace_file_waits_for_a_scanner_to_let_go() {
        let dir = std::env::temp_dir().join(format!("zajil-retry-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let (tmp, state) = (dir.join("x.zajil-state.tmp"), dir.join("x.zajil-state"));
        fs::write(&tmp, b"new").unwrap();
        fs::write(&state, b"old").unwrap();

        // Read access without FILE_SHARE_DELETE, like a scanner.
        let held = open_like_a_scanner(&state).unwrap();
        // Without the retry the rename fails right away.
        assert!(imp::replace_file(&tmp, &state).is_err());

        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(held);
        });
        let started = Instant::now();
        replace_file(&tmp, &state).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert_eq!(fs::read(&state).unwrap(), b"new");
        release.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}
