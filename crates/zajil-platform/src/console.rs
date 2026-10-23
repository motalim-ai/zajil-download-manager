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

#[cfg(unix)]
mod imp {
    pub fn enable_ansi_on_stderr() -> bool {
        true
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        CONSOLE_MODE, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle,
        STD_ERROR_HANDLE, SetConsoleMode,
    };

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
}
