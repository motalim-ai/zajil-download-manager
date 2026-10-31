//! The user's session is ending: a shutdown, a restart or a logoff (#49).
//!
//! On Linux and macOS the program is sent SIGTERM (and on macOS the quit
//! event), which the desktop app already handles. Windows sends no signal
//! to a program with a window: it sends that window `WM_QUERYENDSESSION`,
//! then `WM_ENDSESSION`, and ends the process once the window has answered
//! the second. [`on_session_end`] runs a function inside that answer, so
//! the downloads can record their progress and stop before the process
//! goes. [`watch_session_end`] does the same on a hidden window of its own,
//! so the end is heard while the program shows no window at all (the
//! desktop app frees its main window in the tray). Elsewhere both do
//! nothing.
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

/// Runs `on_end` when the session ends, as [`on_session_end`] does, on a
/// hidden window of its own (never shown, no web page), with its own thread:
/// it is there for as long as the program runs, whatever windows come and
/// go. Windows sends the session end to every top-level window, hidden ones
/// included. A close sent to that window (`WM_CLOSE`) is ignored: only the
/// program's own windows close it.
#[cfg(windows)]
pub fn watch_session_end(on_end: Box<dyn Fn() + Send + 'static>) -> io::Result<()> {
    imp::watch_session_end(on_end)
}

/// Does nothing: Linux and macOS send SIGTERM instead.
#[cfg(not(windows))]
pub fn watch_session_end(_on_end: Box<dyn Fn() + Send + 'static>) -> io::Result<()> {
    Ok(())
}

/// Ends this process at once, with code 0, without its usual exit: at a
/// session end that exit waited seconds for WebView2 to come down (#81).
/// For the desktop app, once everything is saved.
#[cfg(windows)]
pub fn terminate() -> ! {
    imp::terminate()
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::io;

    use windows_sys::Win32::Foundation::{
        ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, RegisterClassW,
        TranslateMessage, WM_CLOSE, WM_ENDSESSION, WM_QUERYENDSESSION, WNDCLASSW, WS_OVERLAPPED,
    };

    /// Our subclass's id on the window (any value unique to us).
    const ID: usize = 0x7a61_6a69; // "zaji"

    type OnEnd = Box<dyn Fn() + Send + 'static>;

    pub fn terminate() -> ! {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        // SAFETY: the pseudo-handle of this process, always valid; the
        // call ends the process and does not return.
        unsafe {
            TerminateProcess(GetCurrentProcess(), 0);
        }
        // Only if Windows refused: the usual exit.
        std::process::exit(0)
    }

    pub fn watch_session_end(on_end: OnEnd) -> io::Result<()> {
        let (made, wait) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("zajil-session-end".into())
            .spawn(move || {
                let window = match hidden_window() {
                    Ok(window) => window,
                    Err(err) => {
                        let _ = made.send(Err(err));
                        return;
                    }
                };
                // On this thread, which owns the window: its messages come
                // through the loop below.
                let _ = made.send(on_session_end(window as isize, on_end));
                // SAFETY: a zeroed MSG is a valid value to be filled in;
                // the loop runs on the thread that made the window, and
                // ends when the message queue does (GetMessageW 0 or -1).
                unsafe {
                    let mut message: MSG = std::mem::zeroed();
                    while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
            })?;
        wait.recv()
            .map_err(|_| io::Error::other("the session-end window was not made"))?
    }

    /// A top-level window that is never shown: no owner (an owned window
    /// would go with its owner), no parent (a child hears no session end),
    /// not message-only (those hear no broadcasts).
    fn hidden_window() -> io::Result<HWND> {
        let class: Vec<u16> = "ZajilSessionEnd\0".encode_utf16().collect();
        let title: Vec<u16> = "Zajil\0".encode_utf16().collect();
        // SAFETY: GetModuleHandleW(null) is this program's own module,
        // valid for as long as it runs. The class and title are
        // NUL-terminated UTF-16 that outlive the calls that read them
        // (Windows copies the class name when it registers it).
        // `hidden_proc` matches WNDPROC; a zeroed WNDCLASSW is a valid start.
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let mut wc: WNDCLASSW = std::mem::zeroed();
            wc.lpfnWndProc = Some(hidden_proc);
            wc.hInstance = instance;
            wc.lpszClassName = class.as_ptr();
            if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(io::Error::last_os_error());
            }
            let window = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if window.is_null() {
                return Err(io::Error::last_os_error());
            }
            Ok(window)
        }
    }

    /// The hidden window's own procedure: a close is ignored, so the window
    /// stays for the program's life; anything else as Windows does it.
    unsafe extern "system" fn hidden_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_CLOSE {
            return 0;
        }
        // SAFETY: the arguments are the ones this procedure received.
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

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
