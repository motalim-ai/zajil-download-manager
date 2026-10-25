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
}
