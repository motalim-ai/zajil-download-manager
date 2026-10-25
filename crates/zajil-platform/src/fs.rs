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

/// Makes `dir` a folder that no other user can change, creating it (and
/// its missing parents) if needed: mode `0700` on macOS and Linux; on
/// Windows the access list the folder inherits from the user's own data
/// folder. An existing `dir` must already be one (checked with
/// [`only_user_or_root_can_change`], never through a link). For the video
/// tools, which Zajil runs (ADR-0009), and the folders of each run.
pub fn private_dir(dir: &Path) -> io::Result<()> {
    imp::create_private_dir(dir)?;
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() || !only_user_or_root_can_change(dir)? {
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

/// Lets the owner run `file` (mode `0700` on macOS and Linux; nothing to do
/// on Windows, where a `.exe` runs by its name). For a tool Zajil checked
/// before saving it (ADR-0009).
pub fn make_executable(file: &File) -> io::Result<()> {
    imp::make_executable(file)
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
/// - **Linux**: `fallocate` reserves the space for real. File systems
///   without it get a plain `set_len` (a sparse file).
/// - **macOS**: `F_PREALLOCATE` reserves the space where the file system
///   supports it, then `set_len`.
/// - **Windows**: the file is marked sparse, then `set_len`. Nothing is
///   reserved: on NTFS a plain `set_len` would make every write far from the
///   start fill the gap before it with zeros first, writing a split download
///   about twice. Callers check [`available_space`] before starting instead
///   (owner decision, M2).
///
/// File systems that cannot have holes (FAT and exFAT) may write zeros up to
/// `len`, and FAT32 refuses files of 4 GiB or more; both are errors from the
/// system, reported as they are. A `file` that is not empty is refused.
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
mod imp {
    use std::ffi::CString;
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
    use std::os::unix::io::AsRawFd;
    use std::path::Path;

    use super::{check_then_rename, not_regular};

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

    pub fn create_private_dir(dir: &Path) -> io::Result<()> {
        use std::os::unix::fs::DirBuilderExt;
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match std::fs::DirBuilder::new().mode(0o700).create(dir) {
            Err(err) if err.kind() != io::ErrorKind::AlreadyExists => Err(err),
            _ => Ok(()),
        }
    }

    pub fn make_executable(file: &File) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o700))
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
        // A second name is a hard link. No name at all (0) is a file that
        // was replaced or removed after we opened it (a reader racing a
        // safe save): no link, so it is allowed.
        if !meta.file_type().is_file() || meta.nlink() > 1 {
            return Err(not_regular(path));
        }
        Ok(file)
    }

    pub fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    pub fn create_private(path: &Path) -> io::Result<File> {
        // The mode applies at creation; the umask can only remove bits.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        // On macOS a file also inherits the folder's inheritable access
        // list, whatever its mode (review of #30): remove it before any
        // byte is written.
        if let Err(err) = acl::remove(&file) {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(err);
        }
        Ok(file)
    }

    #[allow(unsafe_code)]
    pub fn only_user_or_root_can_change(path: &Path) -> io::Result<bool> {
        let meta = std::fs::symlink_metadata(path)?;
        // SAFETY: geteuid has no preconditions and cannot fail.
        let me = unsafe { libc::geteuid() };
        Ok((meta.uid() == me || meta.uid() == 0) && meta.mode() & 0o022 == 0)
    }

    pub fn only_owner_can_access(path: &Path) -> io::Result<bool> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let meta = file.metadata()?;
        Ok(meta.file_type().is_file() && meta.mode() & 0o077 == 0 && !acl::has_entries(&file)?)
    }

    pub fn open_like_a_scanner(path: &Path) -> io::Result<File> {
        File::open(path)
    }

    pub fn write_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
        file.write_all_at(buf, offset)
    }

    /// macOS access lists (ACLs). A file inherits the inheritable entries
    /// of its folder, and those can let others read it whatever its mode.
    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    mod acl {
        use std::ffi::{c_int, c_void};
        use std::fs::File;
        use std::io;
        use std::os::unix::io::AsRawFd;

        /// `acl_type_t` for the extended (NFSv4-style) list macOS uses.
        const ACL_TYPE_EXTENDED: c_int = 0x0000_0100;
        const ACL_FIRST_ENTRY: c_int = 0;

        // From <sys/acl.h> in the macOS SDK (libSystem).
        unsafe extern "C" {
            fn acl_init(count: c_int) -> *mut c_void;
            fn acl_free(object: *mut c_void) -> c_int;
            fn acl_get_fd_np(fd: c_int, kind: c_int) -> *mut c_void;
            fn acl_set_fd_np(fd: c_int, acl: *mut c_void, kind: c_int) -> c_int;
            fn acl_get_entry(acl: *mut c_void, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
        }

        /// Gives `file` an empty access list (like `chmod -N`).
        pub fn remove(file: &File) -> io::Result<()> {
            // SAFETY: asks for an empty list; checked for null below.
            let empty = unsafe { acl_init(0) };
            if empty.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: the descriptor is open for the lifetime of `file`, and
            // `empty` is a valid list, freed once after the call.
            let result = unsafe { acl_set_fd_np(file.as_raw_fd(), empty, ACL_TYPE_EXTENDED) };
            let err = io::Error::last_os_error();
            // SAFETY: `empty` came from `acl_init` and is freed once.
            unsafe { acl_free(empty) };
            if result != 0 && !unsupported(&err) {
                return Err(err);
            }
            Ok(())
        }

        /// File systems without access lists (FAT32 and exFAT on macOS)
        /// refuse the calls: nothing can be inherited there (found by CI
        /// on the FAT32 test volume).
        fn unsupported(err: &io::Error) -> bool {
            matches!(
                err.raw_os_error(),
                Some(libc::ENOTSUP) | Some(libc::EOPNOTSUPP)
            )
        }

        /// Whether `file` has any access list entry at all.
        pub fn has_entries(file: &File) -> io::Result<bool> {
            // SAFETY: the descriptor is open for the lifetime of `file`.
            let list = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
            if list.is_null() {
                let err = io::Error::last_os_error();
                // No list at all, or none possible on this file system.
                return if err.raw_os_error() == Some(libc::ENOENT) || unsupported(&err) {
                    Ok(false)
                } else {
                    Err(err)
                };
            }
            let mut entry = std::ptr::null_mut();
            // SAFETY: `list` is a valid list; `entry` is a writable pointer.
            let found = unsafe { acl_get_entry(list, ACL_FIRST_ENTRY, &mut entry) } == 0;
            // SAFETY: `list` came from `acl_get_fd_np` and is freed once.
            unsafe { acl_free(list) };
            Ok(found)
        }
    }

    /// Linux: a default access list of the folder is masked by the mode, so
    /// `0600` leaves it no effect; nothing to remove.
    #[cfg(not(target_os = "macos"))]
    mod acl {
        use std::fs::File;
        use std::io;

        pub fn remove(_file: &File) -> io::Result<()> {
            Ok(())
        }

        pub fn has_entries(_file: &File) -> io::Result<bool> {
            Ok(false)
        }
    }

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

    #[cfg(target_os = "linux")]
    #[allow(unsafe_code)]
    // The field types are 32 bits on some targets.
    #[allow(clippy::useless_conversion)]
    pub fn available_space(dir: &Path) -> io::Result<u64> {
        let dir_c = c_path(dir)?;
        // SAFETY: an all-zero statvfs is a valid value to be overwritten.
        let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `dir_c` is a valid NUL-terminated string and `stats` a
        // valid, writable structure, both alive for the call.
        if unsafe { libc::statvfs(dir_c.as_ptr(), &mut stats) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(u64::from(stats.f_bavail).saturating_mul(u64::from(stats.f_frsize)))
    }

    #[cfg(target_os = "linux")]
    #[allow(unsafe_code)]
    pub fn allocate(file: &File, len: u64) -> io::Result<()> {
        let len_off = len as libc::off_t; // checked by the caller
        // SAFETY: the descriptor is open for the lifetime of `file`.
        if unsafe { libc::fallocate(file.as_raw_fd(), 0, 0, len_off) } == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            // The file system cannot reserve: a sparse file instead.
            Some(libc::EOPNOTSUPP) | Some(libc::ENOSYS) => file.set_len(len),
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

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    pub fn available_space(dir: &Path) -> io::Result<u64> {
        let dir_c = c_path(dir)?;
        // `statfs`, not `statvfs`: on macOS the latter counts blocks in 32
        // bits and is wrong on large volumes.
        // SAFETY: an all-zero statfs is a valid value to be overwritten.
        let mut stats: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: `dir_c` is a valid NUL-terminated string and `stats` a
        // valid, writable structure, both alive for the call.
        if unsafe { libc::statfs(dir_c.as_ptr(), &mut stats) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(stats.f_bavail.saturating_mul(u64::from(stats.f_bsize)))
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    pub fn allocate(file: &File, len: u64) -> io::Result<()> {
        let length = len as libc::off_t; // checked by the caller
        // Contiguous space if possible, otherwise any space.
        for flags in [
            libc::F_ALLOCATECONTIG | libc::F_ALLOCATEALL,
            libc::F_ALLOCATEALL,
        ] {
            let mut store = libc::fstore_t {
                fst_flags: flags,
                fst_posmode: libc::F_PEOFPOSMODE,
                fst_offset: 0,
                fst_length: length,
                fst_bytesalloc: 0,
            };
            // SAFETY: the descriptor is open for the lifetime of `file`, and
            // `store` is a valid fstore_t that outlives the call.
            if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_PREALLOCATE, &mut store) } != -1 {
                return file.set_len(len);
            }
            let err = io::Error::last_os_error();
            match err.raw_os_error() {
                Some(libc::ENOSPC) if flags & libc::F_ALLOCATECONTIG != 0 => continue,
                // The file system cannot reserve: a sparse file instead.
                Some(libc::ENOTSUP) | Some(libc::EINVAL) => return file.set_len(len),
                _ => return Err(err),
            }
        }
        unreachable!("the last attempt always returns")
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
        link_then_unlink(from, to)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn available_space(_dir: &Path) -> io::Result<u64> {
        Err(io::ErrorKind::Unsupported.into())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn allocate(file: &File, len: u64) -> io::Result<()> {
        file.set_len(len)
    }

    /// `link` fails if `to` exists, so it is a no-replace rename on file
    /// systems with hard links. Where hard links are refused too (FAT and
    /// exFAT), the last resort of `rename_noreplace` follows.
    fn link_then_unlink(from: &Path, to: &Path) -> io::Result<()> {
        match std::fs::hard_link(from, to) {
            Ok(()) => std::fs::remove_file(from),
            Err(err) if links_unsupported(&err) => check_then_rename(from, to),
            Err(err) => Err(err),
        }
    }

    /// Errors with which a file system says it has no hard links: `EPERM`
    /// on Linux (vfat, exfat), `ENOTSUP` or `EOPNOTSUPP` elsewhere.
    pub(super) fn links_unsupported(err: &io::Error) -> bool {
        // ENOTSUP and EOPNOTSUPP are the same number on Linux.
        matches!(err.raw_os_error(), Some(code)
            if code == libc::EPERM || code == libc::ENOTSUP || code == libc::EOPNOTSUPP)
    }
}

#[cfg(windows)]
pub(crate) use imp::{owner_only, runs_as_current_user};

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::{FileExt, MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;

    use std::os::windows::io::{FromRawHandle, RawHandle};

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_FUNCTION, ERROR_LOCK_VIOLATION,
        ERROR_NOT_SUPPORTED, ERROR_SHARING_VIOLATION, GENERIC_READ, GENERIC_WRITE, HANDLE,
        INVALID_HANDLE_VALUE, LocalFree,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation,
        GetSecurityDescriptorControl, GetTokenInformation, INHERIT_ONLY_ACE, IsWellKnownSid,
        OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
        SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser, WinBuiltinAdministratorsSid,
        WinLocalSystemSid,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, GetDiskFreeSpaceExW, GetFileInformationByHandle,
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;
    use windows_sys::Win32::System::SystemServices::{
        ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    use super::not_regular;

    pub fn sync_dir(dir: &Path) -> io::Result<()> {
        std::fs::metadata(dir).map(|_| ())
    }

    pub fn create_private_dir(dir: &Path) -> io::Result<()> {
        // The access list comes from the parent: the user's own data folder.
        std::fs::create_dir_all(dir)
    }

    pub fn make_executable(_file: &File) -> io::Result<()> {
        Ok(())
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
            // 0: replaced or removed after it was opened (see unix).
            || link_count(&file)? > 1
        {
            return Err(not_regular(path));
        }
        Ok(file)
    }

    pub fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    /// Security attributes that allow `access` (SDDL rights, such as `FA`
    /// for all file access or `GA` for all generic access) to the current
    /// user only: "D:P(A;;<access>;;;<sid>)", a protected DACL (P: nothing
    /// inherited) with one entry allowing (A) it. Kept alive by the
    /// returned guard, which must outlive every use of the attributes.
    pub(crate) fn owner_only(access: &str) -> io::Result<(SECURITY_ATTRIBUTES, LocalMemory)> {
        let sddl = format!("D:P(A;;{access};;;{})", UserSid::current()?.to_text()?);
        let sddl: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `sddl` is a NUL-terminated UTF-16 string alive for the
        // call, and `descriptor` a writable pointer; the size is optional.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let guard = LocalMemory(descriptor);
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        Ok((attributes, guard))
    }

    /// Whether the process `pid` runs as the current user (the other end
    /// of a pipe, `zajil_platform::ipc`).
    pub(crate) fn runs_as_current_user(pid: u32) -> io::Result<bool> {
        // SAFETY: plain call; a null handle is checked below.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        let process = Handle(process);
        let (ours, theirs) = (UserSid::current()?, UserSid::of_process(process.0)?);
        // SAFETY: both are valid SIDs, alive for the call.
        Ok(unsafe { EqualSid(ours.as_psid(), theirs.as_psid()) } != 0)
    }

    pub fn create_private(path: &Path) -> io::Result<File> {
        // All file access (FA) for the current user only, given to
        // CreateFileW, so the file never exists with other permissions,
        // not even for an instant.
        let (attributes, _descriptor) = owner_only("FA")?;
        // `absolute` also removes `.` and `..`, which the `\\?\` form of
        // long paths requires.
        let path_w = wide(&std::path::absolute(path)?);
        // SAFETY: `path_w` is a NUL-terminated UTF-16 string and
        // `attributes` points to a valid descriptor, both alive for the
        // call. The sharing mode is the one the standard library uses.
        let handle = unsafe {
            CreateFileW(
                path_w.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                &attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `handle` is a valid file handle that nothing else owns;
        // the `File` closes it.
        Ok(unsafe { File::from_raw_handle(handle as RawHandle) })
    }

    /// The rights that let someone change a file or a folder: write or
    /// add to it, delete it or what is in it, change its permissions or
    /// owner, or anything (generic).
    const CHANGE: u32 = 0x0002 // FILE_WRITE_DATA, FILE_ADD_FILE
        | 0x0004 // FILE_APPEND_DATA, FILE_ADD_SUBDIRECTORY
        | 0x0010 // FILE_WRITE_EA
        | 0x0040 // FILE_DELETE_CHILD
        | 0x0100 // FILE_WRITE_ATTRIBUTES
        | 0x0001_0000 // DELETE
        | 0x0004_0000 // WRITE_DAC
        | 0x0008_0000 // WRITE_OWNER
        | 0x1000_0000 // GENERIC_ALL
        | 0x4000_0000; // GENERIC_WRITE

    /// TrustedInstaller's service SID.
    const TRUSTED_INSTALLER: &str =
        "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

    /// The current user, SYSTEM, the Administrators or TrustedInstaller.
    fn trusted(sid: PSID, user: &UserSid) -> io::Result<bool> {
        // SAFETY: `sid` is a valid SID inside a live descriptor.
        let well_known = unsafe {
            EqualSid(sid, user.as_psid()) != 0
                || IsWellKnownSid(sid, WinLocalSystemSid) != 0
                || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
        };
        if well_known {
            return Ok(true);
        }
        let mut text: *mut u16 = std::ptr::null_mut();
        // SAFETY: `sid` is valid; `text` receives a LocalAlloc'ed string.
        if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let _text = LocalMemory(text.cast());
        // SAFETY: a NUL-terminated UTF-16 string, alive until `_text` drops.
        let len = (0..).take_while(|&i| unsafe { *text.add(i) } != 0).count();
        // SAFETY: `len` UTF-16 units precede the NUL.
        let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) });
        Ok(text == TRUSTED_INSTALLER)
    }

    pub fn only_user_or_root_can_change(path: &Path) -> io::Result<bool> {
        // Not through a link: the link itself is what is checked.
        std::fs::symlink_metadata(path)?;
        let user = UserSid::current()?;
        let path_w = wide(&std::path::absolute(path)?);
        let (mut owner, mut dacl): (PSID, *mut ACL) = (std::ptr::null_mut(), std::ptr::null_mut());
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `path_w` is a NUL-terminated UTF-16 string alive for the
        // call; the outputs are writable pointers, the unused ones null.
        let status = unsafe {
            GetNamedSecurityInfoW(
                path_w.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let _descriptor = LocalMemory(descriptor);
        // Its owner may always change its permissions.
        if owner.is_null() || !trusted(owner, &user)? {
            return Ok(false);
        }
        if dacl.is_null() {
            return Ok(false); // no list at all: everyone may change it
        }
        let mut size = ACL_SIZE_INFORMATION::default();
        // SAFETY: `dacl` points into `descriptor`, alive here; `size` is a
        // writable structure of the size given.
        let ok = unsafe {
            GetAclInformation(
                dacl,
                (&mut size as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        for index in 0..size.AceCount {
            let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
            // SAFETY: `index` is below the entry count of `dacl`.
            if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: every entry starts with an ACE_HEADER.
            let header = unsafe { *(ace as *const ACE_HEADER) };
            // An entry only for what is created inside it does not apply
            // to it.
            if u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0 {
                continue;
            }
            match u32::from(header.AceType) {
                ACCESS_ALLOWED_ACE_TYPE => {
                    let ace = ace as *const ACCESS_ALLOWED_ACE;
                    // SAFETY: an allowing entry has this layout; its SID
                    // starts at SidStart.
                    let (mask, sid) =
                        unsafe { ((*ace).Mask, std::ptr::addr_of!((*ace).SidStart) as PSID) };
                    if mask & CHANGE != 0 && !trusted(sid, &user)? {
                        return Ok(false);
                    }
                }
                // Denying entries only take rights away.
                ACCESS_DENIED_ACE_TYPE => {}
                // Any other kind of allowing entry (object, callback…) is
                // not read: refused.
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    pub fn only_owner_can_access(path: &Path) -> io::Result<bool> {
        let user = UserSid::current()?;
        let path_w = wide(&std::path::absolute(path)?);
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `path_w` is a NUL-terminated UTF-16 string alive for the
        // call; the outputs are writable pointers, the unused ones null.
        let status = unsafe {
            GetNamedSecurityInfoW(
                path_w.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let _descriptor = LocalMemory(descriptor);
        if dacl.is_null() {
            return Ok(false); // no list at all: everyone may open it
        }
        let (mut control, mut revision) = (0u16, 0u32);
        // SAFETY: `descriptor` is the valid descriptor returned above; the
        // outputs are writable.
        if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if control & SE_DACL_PROTECTED == 0 {
            return Ok(false); // entries from the folder may be added later
        }
        let mut size = ACL_SIZE_INFORMATION::default();
        // SAFETY: `dacl` points into `descriptor`, alive here; `size` is a
        // writable structure of the size given.
        let ok = unsafe {
            GetAclInformation(
                dacl,
                (&mut size as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if size.AceCount == 0 {
            return Ok(false);
        }
        for index in 0..size.AceCount {
            let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
            // SAFETY: `index` is below the entry count of `dacl`.
            if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let ace = ace as *const ACCESS_ALLOWED_ACE;
            // SAFETY: every entry starts with an ACE_HEADER; the SID is read
            // only for allowing entries, which have this layout.
            let allows_only_user = unsafe {
                u32::from((*ace).Header.AceType) == ACCESS_ALLOWED_ACE_TYPE
                    && EqualSid(std::ptr::addr_of!((*ace).SidStart) as PSID, user.as_psid()) != 0
            };
            if !allows_only_user {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Memory that a Windows call allocated with `LocalAlloc`.
    pub(crate) struct LocalMemory(*mut core::ffi::c_void);

    impl Drop for LocalMemory {
        fn drop(&mut self) {
            // SAFETY: the pointer came from a call documented to need
            // `LocalFree`, and is freed once.
            unsafe { LocalFree(self.0) };
        }
    }

    /// The security identifier (SID) of the user running this process.
    struct UserSid {
        /// A `TOKEN_USER` and the SID it points to. `u64` keeps it aligned.
        buffer: Vec<u64>,
    }

    impl UserSid {
        fn current() -> io::Result<Self> {
            // SAFETY: the pseudo-handle of this process needs no closing.
            Self::of_process(unsafe { GetCurrentProcess() })
        }

        /// The user a process runs as; `process` must allow
        /// PROCESS_QUERY_LIMITED_INFORMATION.
        fn of_process(process: HANDLE) -> io::Result<Self> {
            let mut token: HANDLE = std::ptr::null_mut();
            // SAFETY: `process` is a process handle with query access;
            // `token` is a writable handle.
            if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let token = Handle(token);
            let mut needed = 0u32;
            // SAFETY: asks only for the size: no buffer, length 0.
            unsafe {
                GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut needed)
            };
            let mut buffer = vec![0u64; (needed as usize).div_ceil(8).max(1)];
            // SAFETY: `buffer` has at least `needed` writable bytes.
            let ok = unsafe {
                GetTokenInformation(
                    token.0,
                    TokenUser,
                    buffer.as_mut_ptr().cast(),
                    needed,
                    &mut needed,
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(UserSid { buffer })
        }

        fn as_psid(&self) -> PSID {
            // SAFETY: the buffer holds the TOKEN_USER written by
            // GetTokenInformation; its SID lives in the same buffer.
            unsafe { (*(self.buffer.as_ptr() as *const TOKEN_USER)).User.Sid }
        }

        /// The SID as text, such as `S-1-5-21-…`.
        fn to_text(&self) -> io::Result<String> {
            let mut text: *mut u16 = std::ptr::null_mut();
            // SAFETY: `as_psid` is a valid SID; `text` a writable pointer.
            if unsafe { ConvertSidToStringSidW(self.as_psid(), &mut text) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let _text = LocalMemory(text.cast());
            // SAFETY: the call returned a NUL-terminated UTF-16 string.
            let len = (0..).take_while(|&i| unsafe { *text.add(i) } != 0).count();
            // SAFETY: `len` characters before the NUL are readable.
            let units = unsafe { std::slice::from_raw_parts(text, len) };
            Ok(String::from_utf16_lossy(units))
        }
    }

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle was opened by this module and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    pub fn open_like_a_scanner(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)
    }

    pub fn write_at(file: &File, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
        while !buf.is_empty() {
            match file.seek_write(buf, offset) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => {
                    buf = &buf[n..];
                    offset = offset
                        .checked_add(n as u64)
                        .ok_or(io::ErrorKind::FileTooLarge)?;
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }

    pub fn available_space(dir: &Path) -> io::Result<u64> {
        let dir_w = wide(dir);
        let mut available = 0u64;
        // SAFETY: `dir_w` is a NUL-terminated UTF-16 string and `available`
        // a writable u64, both alive for the call; the other outputs are
        // optional and passed as null.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                dir_w.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(available)
    }

    pub fn allocate(file: &File, len: u64) -> io::Result<()> {
        let mut returned = 0u32;
        // SAFETY: the handle is open for the lifetime of `file`; this
        // control code takes no input or output buffer, and `returned` is a
        // writable u32 alive for the call. No OVERLAPPED: the call is
        // synchronous.
        let ok = unsafe {
            DeviceIoControl(
                file.as_raw_handle() as HANDLE,
                FSCTL_SET_SPARSE,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            let err = io::Error::last_os_error();
            // FAT and exFAT have no sparse files; the file is simply longer.
            let unsupported = matches!(
                err.raw_os_error().map(|code| code as u32),
                Some(ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED)
            );
            if !unsupported {
                return Err(err);
            }
        }
        file.set_len(len)
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

    /// Review #84: on Windows the check was `true` whatever the
    /// permissions. A folder of the user's own is trusted, and a program in
    /// it; once others may change the folder (as under `C:\`, where
    /// Authenticated Users may modify), or only the program, neither is;
    /// letting others read is fine.
    #[cfg(windows)]
    #[test]
    fn others_who_may_change_a_program_or_its_folder_are_seen_on_windows() {
        let icacls = |path: &Path, grant: &str| {
            let status = std::process::Command::new("icacls")
                .arg(path)
                .args(["/grant", grant])
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "icacls {grant}");
        };
        let mine = tempdir();
        let program = mine.as_path().join("zajil-host.exe");
        fs::write(&program, b"MZ").unwrap();
        assert!(only_user_or_root_can_change(mine.as_path()).unwrap());
        assert!(only_user_or_root_can_change(&program).unwrap());

        // Everyone may read and run: still only the user can change it.
        icacls(&program, "*S-1-1-0:(RX)");
        assert!(only_user_or_root_can_change(&program).unwrap());

        // Everyone may write to the program alone.
        icacls(&program, "*S-1-1-0:(W)");
        assert!(!only_user_or_root_can_change(&program).unwrap());

        // Authenticated Users may modify the folder, and what is made in it.
        let shared = tempdir();
        icacls(shared.as_path(), "*S-1-5-11:(OI)(CI)(M)");
        let program = shared.as_path().join("zajil-host.exe");
        fs::write(&program, b"MZ").unwrap();
        assert!(!only_user_or_root_can_change(shared.as_path()).unwrap());
        assert!(!only_user_or_root_can_change(&program).unwrap());
        let _ = fs::remove_dir_all(&mine);
        let _ = fs::remove_dir_all(&shared);
    }

    /// The same on macOS and Linux: writable by others, or not.
    #[cfg(unix)]
    #[test]
    fn others_who_may_change_a_program_or_its_folder_are_seen_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir();
        let program = dir.as_path().join("zajil-host");
        fs::write(&program, b"#!").unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(dir.as_path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(only_user_or_root_can_change(dir.as_path()).unwrap());
        assert!(only_user_or_root_can_change(&program).unwrap());
        fs::set_permissions(&program, fs::Permissions::from_mode(0o757)).unwrap();
        assert!(!only_user_or_root_can_change(&program).unwrap());
        fs::set_permissions(dir.as_path(), fs::Permissions::from_mode(0o775)).unwrap();
        assert!(!only_user_or_root_can_change(dir.as_path()).unwrap());
        let _ = fs::remove_dir_all(&dir);
    }

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

    /// #55: the name is free at once, even while a handle on the file is
    /// open (as an antivirus holds it, sharing its deletion), and nothing
    /// is ever replaced.
    #[test]
    fn move_aside_frees_the_name_while_the_file_is_held() {
        let dir = tempdir();
        let path = dir.join("f.bin.zajil-part");
        std::fs::write(&path, b"held").unwrap();
        let held = File::open(&path).unwrap();
        let aside = move_aside(&path).unwrap();
        assert_eq!(aside.parent(), Some(dir.as_path()));
        assert!(
            aside
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".zajil-removed-")
        );
        // The name is free for a new file at once.
        let new = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path);
        assert!(new.is_ok(), "{new:?}");
        assert_eq!(std::fs::read(&aside).unwrap(), b"held");
        std::fs::remove_file(&aside).unwrap();
        drop(held);
        // A missing file is NotFound; a second move takes another name.
        assert_eq!(
            move_aside(&dir.join("none")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        let other = move_aside(&path).unwrap();
        assert_ne!(other, aside);
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

    #[test]
    fn write_at_fills_pieces_in_any_order_through_the_locked_handle() {
        let dir = tempdir();
        let path = dir.join("part");
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        // The download's lock: on Windows it belongs to this handle, so the
        // writes must go through it.
        file.try_lock().unwrap();
        allocate(&file, 12).unwrap();
        write_at(&file, b"89ab", 8).unwrap();
        write_at(&file, b"0123", 0).unwrap();
        write_at(&file, b"4567", 4).unwrap();
        file.sync_all().unwrap();
        // While the lock is held, Windows refuses even reads through another
        // handle: the reason every connection writes through this one.
        if cfg!(windows) {
            assert!(fs::read(&path).is_err());
        }
        drop(file);
        assert_eq!(fs::read(&path).unwrap(), b"0123456789ab");
    }

    #[test]
    fn allocate_gives_the_length_and_reads_back_zeros() {
        let dir = tempdir();
        let path = dir.join("part");
        let file = fs::File::create(&path).unwrap();
        allocate(&file, 1 << 20).unwrap();
        drop(file);
        let data = fs::read(&path).unwrap();
        assert_eq!(data.len(), 1 << 20);
        assert!(data.iter().all(|&b| b == 0));
    }

    #[test]
    fn allocate_refuses_a_file_that_is_not_empty() {
        let dir = tempdir();
        let path = dir.join("part");
        fs::write(&path, b"data").unwrap();
        let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        let err = allocate(&file, 10).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(fs::read(&path).unwrap(), b"data");
    }

    #[test]
    fn allocate_refuses_absurd_sizes_without_panicking() {
        let dir = tempdir();
        let file = fs::File::create(dir.join("part")).unwrap();
        for len in [u64::MAX, i64::MAX as u64 + 1] {
            let err = allocate(&file, len).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::FileTooLarge, "{len}");
        }
        // Valid as a number, but no disk is that large.
        assert!(allocate(&file, i64::MAX as u64).is_err());
        assert_eq!(file.metadata().unwrap().len(), 0);
    }

    #[test]
    fn allocate_of_zero_is_an_empty_file() {
        let dir = tempdir();
        let file = fs::File::create(dir.join("part")).unwrap();
        allocate(&file, 0).unwrap();
        assert_eq!(file.metadata().unwrap().len(), 0);
    }

    #[test]
    fn reports_available_space() {
        let dir = tempdir();
        let free = available_space(&dir).unwrap();
        assert!(free > 0);
        assert!(available_space(&dir.join("missing")).is_err());
    }

    #[test]
    fn check_then_rename_refuses_anything_at_the_target() {
        let dir = tempdir();
        let (a, b) = (dir.join("a"), dir.join("b"));
        fs::write(&a, b"new").unwrap();
        fs::write(&b, b"old").unwrap();
        let err = check_then_rename(&a, &b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&a).unwrap(), b"new");
        assert_eq!(fs::read(&b).unwrap(), b"old");

        // A dangling symbolic link counts as taken, and is left alone.
        let link = dir.join("link");
        if symlinks_available(&dir.join("nowhere"), &link) {
            let err = check_then_rename(&a, &link).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
            assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
            assert!(!dir.join("nowhere").exists());
        }

        let c = dir.join("c");
        check_then_rename(&a, &c).unwrap();
        assert_eq!(fs::read(&c).unwrap(), b"new");
        assert!(!a.exists());
    }

    #[cfg(unix)]
    #[test]
    fn only_link_errors_that_mean_no_hard_links_choose_the_last_resort() {
        for code in [libc::EPERM, libc::ENOTSUP, libc::EOPNOTSUPP] {
            assert!(imp::links_unsupported(&io::Error::from_raw_os_error(code)));
        }
        for code in [libc::EEXIST, libc::EACCES, libc::ENOSPC, libc::EXDEV] {
            assert!(!imp::links_unsupported(&io::Error::from_raw_os_error(code)));
        }
    }

    /// On the file systems CI and developers use (ext4, APFS) the space is
    /// really reserved, not a sparse file.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn allocate_reserves_blocks_on_linux_and_macos() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempdir();
        let file = fs::File::create(dir.join("part")).unwrap();
        let len = 8 << 20;
        allocate(&file, len).unwrap();
        let meta = file.metadata().unwrap();
        assert_eq!(meta.len(), len);
        assert!(meta.blocks() * 512 >= len, "{} blocks", meta.blocks());
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

    #[test]
    fn private_files_are_for_the_current_user_only() {
        use std::io::Write;
        let dir = tempdir();
        let path = dir.join("state");
        let mut file = create_private(&path).unwrap();
        file.write_all(b"?sig=SECRET").unwrap();
        drop(file);
        assert_eq!(fs::read(&path).unwrap(), b"?sig=SECRET");
        assert!(only_owner_can_access(&path).unwrap());
        // Like `create_new`: never opens what is already there.
        assert_eq!(
            create_private(&path).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        // Replacing another file (as a state save does) keeps it private.
        let old = dir.join("old");
        fs::write(&old, b"old").unwrap();
        replace_file(&path, &old).unwrap();
        assert!(only_owner_can_access(&old).unwrap());
        // Control: a file made the ordinary way is readable by others, so
        // the check can say no.
        let plain = dir.join("plain");
        fs::write(&plain, b"x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&plain, fs::Permissions::from_mode(0o644)).unwrap();
        }
        assert!(!only_owner_can_access(&plain).unwrap());
        check_with_system_tool(&old);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Review of #30: on macOS a folder's inheritable access list made a
    /// `0600` state file readable by everyone, and the check said it was
    /// private. The test gives the folder such a list with `chmod +a`, the
    /// way the reviewer did, and reads the result with `ls -le`.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_inherited_access_list_is_removed_on_macos() {
        use std::process::Command;
        let dir = tempdir();
        let status = Command::new("/bin/chmod")
            .args(["+a", "everyone allow read,file_inherit"])
            .arg(&dir)
            .status()
            .unwrap();
        assert!(status.success());
        // Control: a file made the ordinary way inherits it, and the check
        // sees it even with mode 0600.
        let plain = dir.join("plain");
        fs::write(&plain, b"x").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&plain, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let listing = |path: &Path| {
            let out = Command::new("/bin/ls")
                .arg("-le")
                .arg(path)
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        assert!(listing(&plain).contains("everyone"), "{}", listing(&plain));
        assert!(!only_owner_can_access(&plain).unwrap());

        let private = dir.join("state");
        drop(create_private(&private).unwrap());
        assert!(
            !listing(&private).contains("everyone"),
            "{}",
            listing(&private)
        );
        assert!(only_owner_can_access(&private).unwrap());
        // Still private after it replaces another file, as a save does.
        let old = dir.join("old");
        fs::write(&old, b"old").unwrap();
        replace_file(&private, &old).unwrap();
        assert!(!listing(&old).contains("everyone"), "{}", listing(&old));
        assert!(only_owner_can_access(&old).unwrap());
        let _ = fs::remove_dir_all(&dir);
    }

    /// Reads the permissions of `path` without our own code: the mode from
    /// the standard library on macOS and Linux, and `icacls` on Windows,
    /// which must list one entry, full access for `whoami`, not inherited.
    #[cfg(unix)]
    fn check_with_system_tool(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{mode:o}");
    }

    #[cfg(windows)]
    fn check_with_system_tool(path: &Path) {
        use std::process::Command;
        let run = |command: &mut Command| {
            let out = command.output().unwrap();
            assert!(out.status.success(), "{out:?}");
            String::from_utf8_lossy(&out.stdout).to_lowercase()
        };
        let user = run(&mut Command::new("whoami")).trim().to_owned();
        let listing = run(Command::new("icacls").arg(path));
        let entries: Vec<&str> = listing.lines().filter(|l| l.contains(":(")).collect();
        assert_eq!(entries.len(), 1, "{listing}");
        assert!(
            entries[0].contains(&format!("{user}:(f)")),
            "{user}: {listing}"
        );
        assert!(!listing.contains("(i)"), "inherited entry: {listing}");
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

    /// Owner decision (M2): a sparse file on Windows, so writes far from the
    /// start do not fill the gap with zeros first.
    #[test]
    fn allocate_makes_a_sparse_file_on_windows() {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_SPARSE_FILE;
        let dir = std::env::temp_dir().join(format!("zajil-sparse-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = fs::File::create(dir.join("part")).unwrap();
        allocate(&file, 64 << 20).unwrap();
        let meta = file.metadata().unwrap();
        assert_eq!(meta.len(), 64 << 20);
        assert_ne!(meta.file_attributes() & FILE_ATTRIBUTE_SPARSE_FILE, 0);
        drop(file);
        let _ = fs::remove_dir_all(&dir);
    }

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
