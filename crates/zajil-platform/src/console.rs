//! Terminal features that differ between systems.

/// Makes the terminal on standard error understand ANSI escape codes (used
/// to redraw the progress line). Returns whether it does; when it does not,
/// callers must not write escape codes.
///
/// Terminals on macOS and Linux always do. On Windows, Windows Terminal
/// does, while the older console (conhost: cmd and PowerShell 5 in their own
/// window) needs `ENABLE_VIRTUAL_TERMINAL_PROCESSING`, which this turns on.
pub fn enable_ansi_on_stderr() -> bool {
    imp::enable_ansi_on_stderr()
}

/// The most a log file holds before it is moved to `<name>.1` (the one
/// before it goes) and a new one starts.
pub const LOG_LIMIT: u64 = 2 * 1024 * 1024;

/// Sends what the program writes to standard error to the end of `file`,
/// when standard error goes nowhere: a program started from the Finder, the
/// Start menu or a desktop file, whose lines were lost (owner's report,
/// 2026-10-29: why the video tools were asked for again could not be
/// told). A terminal or a pipe keeps them (tests read them). The folder is
/// made, a file past [`LOG_LIMIT`] is moved to `<name>.1`, and the file is
/// one only the user can read. Returns whether they now go there.
pub fn log_stderr_to(file: &std::path::Path) -> std::io::Result<bool> {
    if !imp::stderr_goes_nowhere() {
        return Ok(false);
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::metadata(file).is_ok_and(|m| m.len() > LOG_LIMIT) {
        let mut old = file.as_os_str().to_owned();
        old.push(".1");
        std::fs::rename(file, old)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    imp::stderr_to(options.open(file)?)?;
    Ok(true)
}

#[cfg(unix)]
#[allow(unsafe_code)]
mod imp {
    use std::os::fd::AsRawFd;

    pub fn enable_ansi_on_stderr() -> bool {
        true
    }

    /// Standard error is closed, or is `/dev/null`.
    pub fn stderr_goes_nowhere() -> bool {
        // SAFETY: fstat writes into a stat it is given, for a descriptor
        // number; a closed one fails, which is checked.
        let mut err: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(2, &mut err) } != 0 {
            return true;
        }
        std::fs::metadata("/dev/null").is_ok_and(|null| {
            use std::os::unix::fs::MetadataExt;
            err.st_dev as u64 == null.dev() && err.st_ino as u64 == null.ino()
        })
    }

    pub fn stderr_to(file: std::fs::File) -> std::io::Result<()> {
        // SAFETY: both are open descriptors; dup2 makes 2 a copy of the
        // file's, which stays open as long as the process (the file is
        // left open on purpose).
        if unsafe { libc::dup2(file.as_raw_fd(), 2) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        CONSOLE_MODE, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle,
        STD_ERROR_HANDLE, SetConsoleMode, SetStdHandle,
    };

    /// A program without a console has no standard error handle.
    pub fn stderr_goes_nowhere() -> bool {
        // SAFETY: as in `enable_ansi_on_stderr`.
        let handle = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
        handle.is_null() || handle == INVALID_HANDLE_VALUE
    }

    pub fn stderr_to(file: std::fs::File) -> std::io::Result<()> {
        use std::os::windows::io::IntoRawHandle;
        // SAFETY: a handle of an open file, given to the process for good
        // (Rust's standard error asks for it at each write).
        if unsafe { SetStdHandle(STD_ERROR_HANDLE, file.into_raw_handle()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn enable_ansi_on_stderr() -> bool {
        // SAFETY: GetStdHandle takes a constant and returns a handle or an
        // invalid/null value, which is checked before use.
        let handle = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut mode: CONSOLE_MODE = 0;
        // SAFETY: `handle` is a valid standard handle and `mode` is a
        // writable CONSOLE_MODE. The call fails if it is not a console.
        if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
            return false;
        }
        if mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0 {
            return true;
        }
        // SAFETY: same console handle; only adds one documented flag.
        unsafe { SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0 }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn can_be_called_without_a_console() {
        // In tests stderr is usually a pipe; the call must not fail or panic.
        let _ = super::enable_ansi_on_stderr();
    }

    /// In tests standard error is a pipe: it keeps its lines, and nothing
    /// is made.
    #[test]
    fn a_pipe_keeps_its_lines() {
        if super::imp::stderr_goes_nowhere() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("logs").join("zajil.log");
        assert!(!super::log_stderr_to(&file).unwrap());
        assert!(!file.exists());
    }
}
