//! The Unix side of [`super`]: macOS, Linux and the other Unix systems.

use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
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
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    // Each missing level, from the top: `create_dir_all` would give the
    // parents the umask's mode (review: a group-writable `tools`).
    let missing: Vec<&Path> = dir
        .ancestors()
        .take_while(|level| std::fs::symlink_metadata(level).is_err())
        .collect();
    for level in missing.into_iter().rev() {
        match std::fs::DirBuilder::new().mode(0o700).create(level) {
            Ok(()) => {
                let made = open_dir_nofollow(level)?;
                // The umask may have taken bits away, never added any;
                // and on macOS the folder inherited its parent's
                // inheritable access list.
                made.set_permissions(std::fs::Permissions::from_mode(0o700))?;
                acl::remove(&made)?;
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// A folder, opened without following a link.
fn open_dir_nofollow(dir: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(dir)
}

pub fn dir_has_acl(dir: &Path) -> io::Result<bool> {
    acl::has_entries(&open_dir_nofollow(dir)?)
}

#[allow(unsafe_code)]
pub fn open_private_tool(path: &Path) -> io::Result<File> {
    let file = open_regular_nofollow(path, false)?;
    let meta = file.metadata()?;
    // SAFETY: geteuid has no preconditions and cannot fail.
    let me = unsafe { libc::geteuid() };
    let private = (meta.uid() == me || meta.uid() == 0)
        && meta.mode() & 0o022 == 0
        && !acl::has_entries(&file)?;
    if !private {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} can be changed by another user", path.display()),
        ));
    }
    Ok(file)
}

pub fn make_executable(file: &File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o700))
}

pub fn make_ordinary(file: &File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o644))
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

pub fn symlink_dir(target: &Path, link: &Path) -> io::Result<()> {
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
pub fn others_only_add_in(dir: &Path) -> io::Result<bool> {
    let meta = std::fs::symlink_metadata(dir)?;
    // SAFETY: geteuid has no preconditions and cannot fail.
    let me = unsafe { libc::geteuid() };
    Ok(meta.is_dir() && (meta.uid() == me || meta.uid() == 0) && meta.mode() & 0o1000 != 0)
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

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    link_then_unlink(from, to)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn available_space(_dir: &Path) -> io::Result<u64> {
    Err(io::ErrorKind::Unsupported.into())
}

pub fn allocate(file: &File, len: u64) -> io::Result<()> {
    // A sparse file: nothing is reserved (see `super::allocate`).
    file.set_len(len)
}

pub fn size_on_disk(path: &Path) -> io::Result<u64> {
    // `st_blocks` counts 512-byte units on macOS and Linux alike.
    Ok(std::fs::symlink_metadata(path)?
        .blocks()
        .saturating_mul(512))
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
