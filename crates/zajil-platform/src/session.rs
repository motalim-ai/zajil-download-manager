//! The user's session is ending: a shutdown, a restart or a logoff (#49).
//!
//! On Linux and macOS the program is sent SIGTERM (and on macOS the quit
//! event), which the desktop app already handles. Windows sends no signal
//! to a program with a window: it sends that window `WM_QUERYENDSESSION`,
//! then `WM_ENDSESSION`, and ends the process once the window has answered
//! the second. [`on_session_end`] runs a function inside that answer, so
//! the downloads can record their progress and stop before the process
//! goes. Elsewhere it does nothing.
//!
//! Tested on Windows in CI by sending the two messages to the app's window
//! during a download (`apps/desktop/src-tauri/tests/close_resume.rs`). A
//! real shutdown, and how long Windows waits during one, cannot be run by
//! a test.

use std::io;

/// Runs `on_end` when the session ends, inside the window's answer to
/// `WM_ENDSESSION` (the process may be ended right after it returns).
/// `window` is the window's handle (`HWND`). `WM_QUERYENDSESSION` is
/// answered "yes": Zajil never holds a shutdown back.
#[cfg(windows)]
pub fn on_session_end(window: isize, on_end: Box<dyn Fn() + Send + 'static>) -> io::Result<()> {
    imp::on_session_end(window, on_end)
}

/// Does nothing: Linux and macOS send SIGTERM instead.
#[cfg(not(windows))]
pub fn on_session_end(_window: isize, _on_end: Box<dyn Fn() + Send + 'static>) -> io::Result<()> {
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::io;

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_ENDSESSION, WM_QUERYENDSESSION};

    /// Our subclass's id on the window (any value unique to us).
    const ID: usize = 0x7a61_6a69; // "zaji"

    type OnEnd = Box<dyn Fn() + Send + 'static>;

    pub fn on_session_end(window: isize, on_end: OnEnd) -> io::Result<()> {
        // Lives as long as the program: the window may get the message at
        // any time until the process ends.
        let data = Box::into_raw(Box::new(on_end)) as usize;
        // SAFETY: `window` is the handle of a window of this process, given
        // by Tauri; `proc` matches SUBCLASSPROC; `data` points to a boxed
        // closure that is never freed. SetWindowSubclass is called on the
        // window's thread (the desktop app calls this from its setup, which
        // runs on the main thread that owns the window).
        let ok = unsafe { SetWindowSubclass(window as HWND, Some(proc), ID, data) };
        if ok == 0 {
            // Freed again: the subclass was not installed.
            // SAFETY: `data` came from Box::into_raw above and was not used.
            drop(unsafe { Box::from_raw(data as *mut OnEnd) });
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    unsafe extern "system" fn proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        data: usize,
    ) -> LRESULT {
        match message {
            WM_QUERYENDSESSION => 1,
            // `wparam` is nonzero when the session really ends (zero when a
            // shutdown was cancelled after the query).
            WM_ENDSESSION if wparam != 0 => {
                // SAFETY: `data` is the boxed closure given to
                // SetWindowSubclass, never freed.
                let on_end = unsafe { &*(data as *const OnEnd) };
                on_end();
                0
            }
            // SAFETY: the arguments are the ones this procedure received.
            _ => unsafe { DefSubclassProc(window, message, wparam, lparam) },
        }
    }
}
