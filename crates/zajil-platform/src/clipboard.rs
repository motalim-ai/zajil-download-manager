//! The system clipboard (`Clipboard`, ADR-0006), read for clipboard
//! monitoring (M5, off unless the user turns it on): the Win32 clipboard on
//! Windows, `NSPasteboard` on macOS, X11 or Wayland (the data-control
//! protocol) on Linux, the last two through `arboard`.
//!
//! Only text is read, and only when the clipboard has changed where the
//! system says so (the sequence number on Windows, `changeCount` on macOS;
//! Linux has no such number, so it is read each time). What to do with it
//! is decided above this crate (`zajil_core::clipboard`); nothing here
//! keeps or prints it. On Windows, what a password manager marks as not for
//! clipboard monitors (`ExcludeClipboardContentFromMonitorProcessing`, or
//! `CanIncludeInClipboardHistory` set to 0) is not read at all.

use std::io;

/// What the clipboard holds, as far as monitoring needs it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Content {
    /// No text: empty, an image or files only, or marked not to be read.
    Empty,
    Text(String),
    /// Text longer than asked for; on Windows not even copied.
    Longer,
}

/// Reads the clipboard's text.
pub trait Clipboard {
    /// Whether the clipboard may have changed since the last call; the
    /// first call says yes. Where the system has no change number, always
    /// yes.
    fn changed(&mut self) -> bool {
        true
    }

    /// The clipboard's text, if it is `max_bytes` long at most (UTF-8).
    fn text(&mut self, max_bytes: usize) -> io::Result<Content>;
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
    fn changed(&mut self) -> bool {
        self.0.changed()
    }

    fn text(&mut self, max_bytes: usize) -> io::Result<Content> {
        self.0.text(max_bytes)
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;

    use super::Content;

    pub struct Inner {
        clipboard: arboard::Clipboard,
        /// macOS: the pasteboard's `changeCount` at the last call.
        #[cfg(target_os = "macos")]
        count: Option<isize>,
    }

    impl Inner {
        pub fn new() -> io::Result<Self> {
            Ok(Inner {
                clipboard: arboard::Clipboard::new().map_err(other)?,
                #[cfg(target_os = "macos")]
                count: None,
            })
        }

        /// macOS: `changeCount` changes with every copy, and reading it
        /// reads nothing of the content. Linux: no such number.
        pub fn changed(&mut self) -> bool {
            #[cfg(target_os = "macos")]
            {
                let count = objc2_app_kit::NSPasteboard::generalPasteboard().changeCount();
                self.count.replace(count) != Some(count)
            }
            #[cfg(not(target_os = "macos"))]
            {
                true
            }
        }

        pub fn set_text(&mut self, text: &str) -> io::Result<()> {
            self.clipboard.set_text(text).map_err(other)
        }

        pub fn clear(&mut self) -> io::Result<()> {
            self.clipboard.clear().map_err(other)
        }

        /// `arboard` reads the whole text; a longer one is dropped at once.
        pub fn text(&mut self, max_bytes: usize) -> io::Result<Content> {
            match self.clipboard.get_text() {
                Ok(text) if text.len() > max_bytes => Ok(Content::Longer),
                Ok(text) => Ok(Content::Text(text)),
                Err(arboard::Error::ContentNotAvailable) => Ok(Content::Empty),
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
        CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
        IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };

    use super::Content;
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
    };
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

    /// The clipboard is opened for each use only: while open, it blocks
    /// every other program.
    pub struct Inner {
        /// `ExcludeClipboardContentFromMonitorProcessing`, registered.
        exclude: u32,
        /// `CanIncludeInClipboardHistory`, registered.
        history: u32,
        /// The clipboard's sequence number at the last call.
        sequence: Option<u32>,
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
            let register = |name: &str| {
                let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
                // SAFETY: `name` is NUL-terminated and alive for the call.
                match unsafe { RegisterClipboardFormatW(name.as_ptr()) } {
                    0 => Err(io::Error::last_os_error()),
                    format => Ok(format),
                }
            };
            Ok(Inner {
                exclude: register("ExcludeClipboardContentFromMonitorProcessing")?,
                history: register("CanIncludeInClipboardHistory")?,
                sequence: None,
            })
        }

        /// The sequence number changes with every change of the clipboard,
        /// and reading it opens nothing.
        pub fn changed(&mut self) -> bool {
            // SAFETY: plain call.
            let sequence = unsafe { GetClipboardSequenceNumber() };
            self.sequence.replace(sequence) != Some(sequence)
        }

        /// `CanIncludeInClipboardHistory` present with the value 0.
        ///
        /// # Safety
        /// The clipboard must be open.
        unsafe fn kept_out_of_history(&self) -> bool {
            // SAFETY: the clipboard is open (the caller's contract); the
            // handle stays owned by it, and is read within its size.
            unsafe {
                if IsClipboardFormatAvailable(self.history) == 0 {
                    return false;
                }
                let data = GetClipboardData(self.history);
                if data.is_null() || GlobalSize(data) < 4 {
                    return false;
                }
                let value = GlobalLock(data) as *const u32;
                if value.is_null() {
                    return false;
                }
                let zero = value.read_unaligned() == 0;
                GlobalUnlock(data);
                zero
            }
        }

        pub fn text(&mut self, max_bytes: usize) -> io::Result<Content> {
            let _open = Open::new()?;
            // SAFETY: plain calls with the clipboard open.
            let (excluded, text) = unsafe {
                (
                    IsClipboardFormatAvailable(self.exclude) != 0 || self.kept_out_of_history(),
                    IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT)) != 0,
                )
            };
            if excluded || !text {
                return Ok(Content::Empty);
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
                // A UTF-8 byte is at most one UTF-16 unit: more units than
                // `max_bytes` (and its NUL) is longer, and is not copied.
                if size > max_bytes.saturating_add(1) {
                    return Ok(Content::Longer);
                }
                let chars = GlobalLock(data) as *const u16;
                if chars.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let all = std::slice::from_raw_parts(chars, size);
                let end = all.iter().position(|&c| c == 0).unwrap_or(all.len());
                let text = String::from_utf16_lossy(&all[..end]);
                GlobalUnlock(data);
                if text.len() > max_bytes {
                    return Ok(Content::Longer);
                }
                Ok(Content::Text(text))
            }
        }

        /// For the test: `OpenClipboard` without a window, then
        /// `EmptyClipboard`, is documented to make `SetClipboardData` fail,
        /// but works on the CI's Windows; the app itself never writes.
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
        clipboard.changed();
        let text = "https://example.com/زاجل/file.zip";
        clipboard.set_text(text).unwrap();
        assert!(clipboard.changed(), "a copy is a change");
        assert_eq!(
            clipboard.text(8192).unwrap(),
            Content::Text(text.to_owned())
        );
        assert_eq!(clipboard.text(10).unwrap(), Content::Longer);
        if cfg!(any(windows, target_os = "macos")) {
            assert!(!clipboard.changed(), "reading is not a change");
        }
        clipboard.clear().unwrap();
        assert!(clipboard.changed());
        assert_eq!(clipboard.text(8192).unwrap(), Content::Empty);
    }
}
