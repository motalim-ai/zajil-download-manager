//! The mark a file downloaded from the internet carries (security review
//! #84): with it the system warns before a program from the internet runs
//! (SmartScreen on Windows, Gatekeeper on macOS), and Office opens such a
//! document in Protected View with its macros blocked. Browsers write it on
//! every download; so does Zajil, on every finished file.
//!
//! - Windows: the `Zone.Identifier` stream, with `ZoneId=3` (the Internet
//!   zone) and `HostUrl`, the link's origin only (no path, no query, which
//!   may hold a token).
//! - macOS: the `com.apple.quarantine` extended attribute.
//! - Linux: `user.xdg.origin.url` (freedesktop's convention, as Chrome
//!   writes it), the origin only.
//!
//! A file system that cannot hold it (FAT32, exFAT, a network share) keeps
//! the file without it: the caller decides what to do with the error.
//!
//! On macOS the system also writes the mark by itself on every file a
//! marked process writes: Chrome marks its own process, and so every
//! program it starts, and theirs. [`clear`] removes it from what Zajil
//! checked to be its own (the video tools).

use std::io;
use std::path::Path;

/// Marks `file` as downloaded from `origin` (`scheme://host[:port]`).
pub fn mark(file: &Path, origin: &str) -> io::Result<()> {
    imp::mark(file, origin)
}

/// The mark on `file`, as the system keeps it, if any: for checks and
/// tests.
pub fn read(file: &Path) -> io::Result<Option<String>> {
    imp::read(file)
}

/// Removes the mark from `path` (a file, a folder, or a link itself, never
/// what it points to). Nothing to remove, or a file system that cannot
/// hold it, is not an error; nor is a file that may not be changed and has
/// no mark (macOS refuses the removal before it looks: review of #283, a
/// tool's folder owned by root).
pub fn clear(path: &Path) -> io::Result<()> {
    match imp::clear(path) {
        Err(err)
            if err.kind() == io::ErrorKind::PermissionDenied && matches!(read(path), Ok(None)) =>
        {
            Ok(())
        }
        done => done,
    }
}

/// Marks this process as Chrome marks its own (`LSFileQuarantineEnabled`):
/// from now on the system marks every file it, or a program it starts,
/// writes, with `agent` as the program that downloaded it. Once marked, a
/// process stays so. For tests only: they stand in for the browser.
#[cfg(all(target_os = "macos", feature = "test-support"))]
#[allow(unsafe_code)]
pub fn quarantine_this_process(agent: &str) -> io::Result<()> {
    use std::ffi::{CString, c_char, c_int, c_void};
    // libquarantine, part of libSystem: what LaunchServices calls for an
    // app that asks for it.
    unsafe extern "C" {
        fn _qtn_proc_alloc() -> *mut c_void;
        fn _qtn_proc_set_identifier(qtn: *mut c_void, identifier: *const c_char) -> c_int;
        fn _qtn_proc_set_flags(qtn: *mut c_void, flags: u32) -> c_int;
        fn _qtn_proc_apply_to_self(qtn: *mut c_void) -> c_int;
        fn _qtn_proc_free(qtn: *mut c_void);
    }
    let agent = CString::new(agent)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a NUL in the name"))?;
    // SAFETY: a handle the library allocated, used and freed here only,
    // and a NUL-terminated string alive for the calls.
    let done = unsafe {
        let qtn = _qtn_proc_alloc();
        if qtn.is_null() {
            return Err(io::Error::other("no quarantine handle"));
        }
        // 0081: "downloaded, not yet opened", as Chrome's files say.
        let mut done = _qtn_proc_set_identifier(qtn, agent.as_ptr());
        if done == 0 {
            done = _qtn_proc_set_flags(qtn, 0x0081);
        }
        if done == 0 {
            done = _qtn_proc_apply_to_self(qtn);
        }
        _qtn_proc_free(qtn);
        done
    };
    if done == 0 {
        Ok(())
    } else {
        Err(io::Error::other(format!("quarantine error {done}")))
    }
}

#[cfg(windows)]
mod imp {
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};

    fn stream(file: &Path) -> PathBuf {
        let mut name = file.as_os_str().to_owned();
        name.push(":Zone.Identifier");
        PathBuf::from(name)
    }

    pub fn mark(file: &Path, origin: &str) -> io::Result<()> {
        let mut out = std::fs::File::create(stream(file))?;
        write!(out, "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl={origin}/\r\n")?;
        out.sync_all()
    }

    pub fn read(file: &Path) -> io::Result<Option<String>> {
        match std::fs::read_to_string(stream(file)) {
            Ok(text) => Ok(Some(text)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub fn clear(path: &Path) -> io::Result<()> {
        match std::fs::remove_file(stream(path)) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        }
    }
}

#[cfg(unix)]
mod imp {
    use std::ffi::CString;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    #[cfg(target_os = "macos")]
    const NAME: &str = "com.apple.quarantine";
    #[cfg(not(target_os = "macos"))]
    const NAME: &str = "user.xdg.origin.url";

    fn value(origin: &str) -> String {
        if cfg!(target_os = "macos") {
            // flags;time (hex seconds);agent: 0081 is "downloaded, not yet
            // opened", as browsers write it.
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            format!("0081;{secs:08x};Zajil;")
        } else {
            format!("{origin}/")
        }
    }

    fn c(path: &Path) -> io::Result<CString> {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a NUL in the path"))
    }

    #[allow(unsafe_code)]
    pub fn mark(file: &Path, origin: &str) -> io::Result<()> {
        let (path, name, value) = (c(file)?, CString::new(NAME).expect("no NUL"), value(origin));
        // SAFETY: NUL-terminated strings and a buffer of the length given,
        // all alive for the call.
        #[cfg(target_os = "macos")]
        let done = unsafe {
            libc::setxattr(
                path.as_ptr(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
                libc::XATTR_NOFOLLOW,
            )
        };
        // SAFETY: as above; `lsetxattr` does not follow a link.
        #[cfg(not(target_os = "macos"))]
        let done = unsafe {
            libc::lsetxattr(
                path.as_ptr(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        };
        if done == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[allow(unsafe_code)]
    pub fn read(file: &Path) -> io::Result<Option<String>> {
        let (path, name) = (c(file)?, CString::new(NAME).expect("no NUL"));
        let mut buffer = vec![0u8; 1024];
        // SAFETY: NUL-terminated strings, and a writable buffer of the
        // length given.
        #[cfg(target_os = "macos")]
        let got = unsafe {
            libc::getxattr(
                path.as_ptr(),
                name.as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                0,
                libc::XATTR_NOFOLLOW,
            )
        };
        // SAFETY: as above.
        #[cfg(not(target_os = "macos"))]
        let got = unsafe {
            libc::lgetxattr(
                path.as_ptr(),
                name.as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        if got < 0 {
            let err = io::Error::last_os_error();
            #[cfg(target_os = "macos")]
            let missing = err.raw_os_error() == Some(libc::ENOATTR);
            #[cfg(not(target_os = "macos"))]
            let missing = err.raw_os_error() == Some(libc::ENODATA);
            return if missing { Ok(None) } else { Err(err) };
        }
        buffer.truncate(got as usize);
        Ok(Some(String::from_utf8_lossy(&buffer).into_owned()))
    }

    #[allow(unsafe_code)]
    pub fn clear(path: &Path) -> io::Result<()> {
        let (path, name) = (c(path)?, CString::new(NAME).expect("no NUL"));
        // SAFETY: NUL-terminated strings alive for the call.
        #[cfg(target_os = "macos")]
        let done = unsafe { libc::removexattr(path.as_ptr(), name.as_ptr(), libc::XATTR_NOFOLLOW) };
        // SAFETY: as above; `lremovexattr` does not follow a link.
        #[cfg(not(target_os = "macos"))]
        let done = unsafe { libc::lremovexattr(path.as_ptr(), name.as_ptr()) };
        if done == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        #[cfg(target_os = "macos")]
        let missing = err.raw_os_error() == Some(libc::ENOATTR);
        #[cfg(not(target_os = "macos"))]
        let missing = err.raw_os_error() == Some(libc::ENODATA);
        if missing || err.raw_os_error() == Some(libc::ENOTSUP) {
            Ok(())
        } else {
            Err(err)
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn clear_removes_the_mark_and_a_file_without_one_is_never_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        if super::mark(&file, "https://example.com").is_ok() {
            assert!(super::read(&file).unwrap().is_some());
            super::clear(&file).unwrap();
            assert_eq!(super::read(&file).unwrap(), None);
        }
        super::clear(&file).unwrap();
        // One that may not be changed and has no mark (review of #283).
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o400)).unwrap();
        super::clear(&file).unwrap();
    }
}
