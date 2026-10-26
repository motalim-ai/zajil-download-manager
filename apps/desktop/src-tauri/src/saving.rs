//! Nothing half-written when Zajil is ended at the end of a Windows session
//! (#81). Windows waits for the window to answer `WM_ENDSESSION`, and the
//! process's own exit then took seconds while WebView2 came down, past
//! what a shutdown waits. So once the downloads have stopped, Zajil ends
//! its process at once (`terminate`), but only after every write in
//! progress is done: the stores' (queues, settings, categories) by their
//! own lock files, the app's own files (`window.json`, `videos.json`) by
//! [`writing`]. Both are then kept until the end, so no write starts
//! after (owner's conditions on #81).

// Only Windows ends a session this way; elsewhere only the tests use it.
#![cfg_attr(not(windows), allow(dead_code))]

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard, TryLockError as TryGuardError};
use std::time::{Duration, Instant};

/// The lock files the stores write under (`QueueStore`, `SettingsStore`,
/// `CategoryStore`), in the data folder.
const STORE_LOCKS: [&str; 3] = ["queues.lock", "settings.lock", "categories.lock"];

/// How often a lock still held is tried again.
const RETRY: Duration = Duration::from_millis(10);

/// Held, shared, while one of the app's own files is written; taken whole
/// by [`hold`].
static WRITES: RwLock<()> = RwLock::new(());

fn writing() -> RwLockReadGuard<'static, ()> {
    WRITES.read().unwrap_or_else(|e| e.into_inner())
}

/// Writes `bytes` to `path` whole: a new file `tmp`, synced, renamed over
/// `path`, then the folder synced.
pub fn write_whole(path: &Path, tmp: &Path, bytes: &[u8]) -> io::Result<()> {
    let _writing = writing();
    let mut file = File::create(tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(tmp, path)?;
    match path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => zajil_platform::fs::sync_dir(folder),
        _ => Ok(()),
    }
}

/// What [`hold`] took: kept until the process ends.
pub struct Held {
    _files: Vec<File>,
    _writes: Option<RwLockWriteGuard<'static, ()>>,
}

/// Waits, until `deadline`, for the writes in progress in the data folder
/// `data` to end, and keeps any new one from starting. Also returns what
/// was still being written at the deadline (none: everything is on disk).
pub fn hold(data: &Path, deadline: Instant) -> (Held, Vec<&'static str>) {
    let mut late = Vec::new();
    let mut files = Vec::new();
    for name in STORE_LOCKS {
        match lock_until(&data.join(name), deadline) {
            Ok(Some(file)) => files.push(file),
            Ok(None) => late.push(name),
            Err(err) => {
                eprintln!("zajil: {name} could not be locked: {err}");
                late.push(name);
            }
        }
    }
    let writes = loop {
        match WRITES.try_write() {
            Ok(guard) => break Some(guard),
            Err(TryGuardError::Poisoned(e)) => break Some(e.into_inner()),
            Err(TryGuardError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(RETRY)
            }
            Err(TryGuardError::WouldBlock) => {
                late.push("window.json/videos.json");
                break None;
            }
        }
    };
    (
        Held {
            _files: files,
            _writes: writes,
        },
        late,
    )
}

/// The lock file `path`, locked as the stores lock it (opened never
/// through a link, created if missing); `None` if still held at
/// `deadline`.
fn lock_until(path: &Path, deadline: Instant) -> io::Result<Option<File>> {
    let file = loop {
        match zajil_platform::fs::open_regular_nofollow(path, true) {
            Ok(file) => break file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => break file,
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
    };
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(file)),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::sleep(RETRY),
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(err)) => return Err(err),
        }
    }
}

/// Ends the process at once, with code 0, without the exit that waits for
/// WebView2 (#81).
#[cfg(windows)]
pub fn terminate() -> ! {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
    // SAFETY: the pseudo-handle of this process, always valid; the call
    // ends it and does not return.
    unsafe {
        TerminateProcess(GetCurrentProcess(), 0);
    }
    // Only if Windows refused: the usual exit.
    std::process::exit(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_store_write_in_progress_is_waited_for_and_none_starts_after() {
        let dir = tempfile::tempdir().unwrap();
        let store = zajil_core::settings::SettingsStore::new(dir.path());
        // A write in progress: the store's lock is held for 300 ms.
        let lock = dir.path().join("settings.lock");
        File::create(&lock).unwrap();
        let writer = File::options().write(true).open(&lock).unwrap();
        writer.lock().unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(writer);
        });
        let asked = Instant::now();
        let (held, late) = hold(dir.path(), asked + Duration::from_secs(5));
        // (`window.json/videos.json` may be late: other tests write theirs.)
        assert!(late.iter().all(|l| l.ends_with(".json")), "{late:?}");
        assert!(asked.elapsed() >= Duration::from_millis(250));
        release.join().unwrap();
        // While held, a new write waits.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = store.update(|s| {
                s.clipboard = true;
                Ok(())
            });
            let _ = tx.send(());
        });
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        drop(held);
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn a_write_still_going_at_the_deadline_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("queues.lock");
        File::create(&lock).unwrap();
        let writer = File::options().write(true).open(&lock).unwrap();
        writer.lock().unwrap();
        let _writing = writing();
        let (_held, late) = hold(dir.path(), Instant::now() + Duration::from_millis(50));
        assert_eq!(late, ["queues.lock", "window.json/videos.json"]);
    }

    #[test]
    fn a_file_is_written_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("window.json");
        write_whole(&path, &path.with_extension("json.tmp"), b"{}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
        assert!(!path.with_extension("json.tmp").exists());
    }
}
