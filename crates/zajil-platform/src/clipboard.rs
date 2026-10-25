//! The system clipboard (`Clipboard`, ADR-0006), read for clipboard
//! monitoring (M5, off unless the user turns it on): the Win32 clipboard on
//! Windows, `NSPasteboard` on macOS, X11 or Wayland (the data-control
//! protocol) on Linux, the last two through `arboard`.
//!
//! Only text is read. What to do with it is decided above this crate
//! (`zajil_core::clipboard`); nothing here keeps or prints it. On Windows,
//! what a password manager marks as not for clipboard monitors
//! (`ExcludeClipboardContentFromMonitorProcessing`) is not read at all.

use std::io;

/// Reads the clipboard's text.
pub trait Clipboard {
    /// The clipboard's text; `None` when it holds none (empty, or an image
    /// or files only).
    fn text(&mut self) -> io::Result<Option<String>>;
}

/// The clipboard of this system.
pub struct SystemClipboard(imp::Inner);

impl SystemClipboard {
    /// Opens the clipboard. On Linux this needs a display: an X11 server,
    /// or a Wayland compositor with the data-control protocol (others go
    /// through XWayland when it is there).
    pub fn new() -> io::Result<Self> {
        imp::Inner::new().map(SystemClipboard)
    }

    /// Puts `text` on the clipboard, replacing what is there.
    pub fn set_text(&mut self, text: &str) -> io::Result<()> {
        self.0.set_text(text)
    }

    /// Empties the clipboard.
    pub fn clear(&mut self) -> io::Result<()> {
        self.0.clear()
    }
}

impl Clipboard for SystemClipboard {
    fn text(&mut self) -> io::Result<Option<String>> {
        self.0.text()
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;

    pub struct Inner(arboard::Clipboard);

    impl Inner {
        pub fn new() -> io::Result<Self> {
            arboard::Clipboard::new().map(Inner).map_err(other)
        }

        pub fn set_text(&mut self, text: &str) -> io::Result<()> {
            self.0.set_text(text).map_err(other)
        }

        pub fn clear(&mut self) -> io::Result<()> {
            self.0.clear().map_err(other)
        }

        pub fn text(&mut self) -> io::Result<Option<String>> {
            match self.0.get_text() {
                Ok(text) => Ok(Some(text)),
                Err(arboard::Error::ContentNotAvailable) => Ok(None),
                Err(err) => Err(other(err)),
            }
        }
    }

    fn other(err: arboard::Error) -> io::Error {
        io::Error::other(format!("the clipboard: {err}"))
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::io;

    use windows_sys::Win32::Foundation::{GlobalFree, HANDLE};
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
        OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
    };
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

    /// The clipboard is opened for each use only: while open, it blocks
    /// every other program.
    pub struct Inner {
        /// `ExcludeClipboardContentFromMonitorProcessing`, registered.
        exclude: u32,
    }

    /// The clipboard open; closed on drop.
    struct Open;

    impl Open {
        fn new() -> io::Result<Open> {
            // SAFETY: a null window: the clipboard is opened for this task.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } == 0 {
                return Err(io::Error::other(format!(
                    "the clipboard is held by another program: {}",
                    io::Error::last_os_error()
                )));
            }
            Ok(Open)
        }
    }

    impl Drop for Open {
        fn drop(&mut self) {
            // SAFETY: opened by `Open::new` on this thread.
            unsafe { CloseClipboard() };
        }
    }

    impl Inner {
        pub fn new() -> io::Result<Self> {
            let name: Vec<u16> = "ExcludeClipboardContentFromMonitorProcessing"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            // SAFETY: `name` is NUL-terminated and alive for the call.
            let exclude = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
            if exclude == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Inner { exclude })
        }

        pub fn text(&mut self) -> io::Result<Option<String>> {
            let _open = Open::new()?;
            // SAFETY: plain calls with the clipboard open.
            let (excluded, text) = unsafe {
                (
                    IsClipboardFormatAvailable(self.exclude) != 0,
                    IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT)) != 0,
                )
            };
            if excluded || !text {
                return Ok(None);
            }
            // SAFETY: the clipboard is open; the handle stays owned by it.
            let data: HANDLE = unsafe { GetClipboardData(u32::from(CF_UNICODETEXT)) };
            if data.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: `data` is the clipboard's global memory, locked for the
            // read and read only within its size.
            unsafe {
                let size = GlobalSize(data) / 2;
                let chars = GlobalLock(data) as *const u16;
                if chars.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let all = std::slice::from_raw_parts(chars, size);
                let end = all.iter().position(|&c| c == 0).unwrap_or(all.len());
                let text = String::from_utf16_lossy(&all[..end]);
                GlobalUnlock(data);
                Ok(Some(text))
            }
        }

        pub fn set_text(&mut self, text: &str) -> io::Result<()> {
            let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            let _open = Open::new()?;
            // SAFETY: the clipboard is open; the memory is allocated, filled
            // within its size and unlocked, then given to the clipboard,
            // which owns it from then on (freed here only if refused).
            unsafe {
                if EmptyClipboard() == 0 {
                    return Err(io::Error::last_os_error());
                }
                let memory = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
                if memory.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let target = GlobalLock(memory) as *mut u16;
                if target.is_null() {
                    let err = io::Error::last_os_error();
                    GlobalFree(memory);
                    return Err(err);
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
                GlobalUnlock(memory);
                if SetClipboardData(u32::from(CF_UNICODETEXT), memory).is_null() {
                    let err = io::Error::last_os_error();
                    GlobalFree(memory);
                    return Err(err);
                }
            }
            Ok(())
        }

        pub fn clear(&mut self) -> io::Result<()> {
            let _open = Open::new()?;
            // SAFETY: the clipboard is open.
            if unsafe { EmptyClipboard() } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text put on the real clipboard is read back, and an emptied one
    /// reads as no text. Ignored by default because it replaces the
    /// clipboard of whoever runs it; CI runs it on the three systems (on
    /// Linux under Xvfb; the step "The real clipboard" in `ci.yml`).
    #[test]
    #[ignore = "replaces the clipboard; run in CI"]
    fn text_put_on_the_clipboard_is_read_back() {
        let mut clipboard = SystemClipboard::new().expect("a clipboard (on Linux, a display)");
        let text = "https://example.com/زاجل/file.zip";
        clipboard.set_text(text).unwrap();
        assert_eq!(clipboard.text().unwrap().as_deref(), Some(text));
        clipboard.clear().unwrap();
        assert_eq!(clipboard.text().unwrap(), None);
    }
}
