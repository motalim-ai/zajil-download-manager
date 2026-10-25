//! A channel between Zajil's own programs on one computer, for the current
//! user only (ADR-0005): `zajil-host`, which the browser starts for the
//! extension, hands each request to the running app through it.
//!
//! | | Where | Who can connect, and to whom |
//! |---|---|---|
//! | macOS, Linux | a Unix socket `<data dir>/ipc/zajil.sock` | the folder is `0700` and this user's, so only they reach the socket, from the moment it exists |
//! | Windows | a named pipe `\\.\pipe\zajil-<random>`, its name written in `<data dir>/zajil.pipe` | the pipe's access list allows the current user only and remote clients are refused; **and the client checks that the pipe's server process runs as the current user** before sending anything |
//!
//! On Windows pipe names are global and can be listed by anyone: the name
//! is not a secret, and another user could create a pipe of that name
//! once Zajil's is gone (the name file outlives a crash). So the client
//! never trusts the name alone: it asks the pipe for its server's process
//! (`GetNamedPipeServerProcessId`) and refuses it unless that process runs
//! as the same user. It also connects at the identification level only
//! (`SECURITY_IDENTIFICATION`), so a server could never act as the user.
//!
//! Whether an app already has the channel is decided by a lock held as
//! long as it runs (`ipc/zajil.lock`, or `zajil.lock` on Windows), never by
//! trying to connect (a child process forked at that moment would look
//! like a running app). A second app on the same data folder is refused
//! with [`io::ErrorKind::AddrInUse`].
//!
//! Each connection carries one request and its answer; the format is the
//! caller's (`zajil-host`'s protocol).

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A connection on the channel, from either side.
pub struct Stream(imp::Stream);

impl Stream {
    /// Gives up a read or write after `timeout` (macOS, Linux). Windows'
    /// pipes have no such setting: callers bound the wait themselves.
    pub fn set_timeout(&self, timeout: Duration) -> io::Result<()> {
        imp::set_timeout(&self.0, timeout)
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// The app's end: waits for connections. Holds the data folder's lock
/// while it lives, and removes its socket or name file when dropped.
pub struct Listener {
    inner: imp::Listener,
    _lock: File,
}

impl Listener {
    /// Opens the channel of the data folder `data_dir`. Fails with
    /// [`io::ErrorKind::AddrInUse`] while another app has it open.
    pub fn bind(data_dir: &Path) -> io::Result<Self> {
        let lock = lock(&imp::lock_path(data_dir)?)?;
        Ok(Listener {
            inner: imp::Listener::bind(data_dir)?,
            _lock: lock,
        })
    }

    /// Waits for the next connection.
    pub fn accept(&self) -> io::Result<Stream> {
        self.inner.accept().map(Stream)
    }
}

/// Takes the data folder's channel lock. A lock held only for an instant
/// (inherited by a child process between its fork and its exec) is waited
/// out; one held longer is another app.
fn lock(path: &Path) -> io::Result<File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "another Zajil is running on this data folder",
                ));
            }
            Err(std::fs::TryLockError::Error(err)) => return Err(err),
        }
    }
}

/// Connects to the app of the data folder `data_dir`, waiting up to
/// `patience` while it is starting or busy. [`io::ErrorKind::NotFound`] or
/// [`io::ErrorKind::ConnectionRefused`]: no app has it open.
pub fn connect(data_dir: &Path, patience: Duration) -> io::Result<Stream> {
    let deadline = Instant::now() + patience;
    loop {
        match imp::connect(data_dir) {
            Ok(stream) => return Ok(Stream(stream)),
            Err(err) if Instant::now() < deadline && retry(&err) => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(err) => return Err(err),
        }
    }
}

/// Worth trying again: not there yet, or busy accepting someone else.
fn retry(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    ) || imp::is_busy(err)
}

/// Where the channel of `data_dir` is found (for messages).
pub fn describe(data_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        data_dir.join("zajil.pipe")
    } else {
        data_dir.join("ipc").join("zajil.sock")
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
mod imp {
    use std::fs::DirBuilder;
    use std::io;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    pub type Stream = UnixStream;

    /// The longest socket path the systems take (`sun_path`: 104 bytes on
    /// macOS, 108 on Linux, with its terminating NUL).
    const MAX_SOCKET_PATH: usize = 103;

    /// `<data dir>/ipc`: made `0700`, or checked to be a folder of this
    /// user that only they can enter (never a link).
    fn folder(data_dir: &Path) -> io::Result<PathBuf> {
        let dir = data_dir.join("ipc");
        match DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
        let meta = std::fs::symlink_metadata(&dir)?;
        // SAFETY: geteuid has no preconditions and cannot fail.
        let me = unsafe { libc::geteuid() };
        if !meta.is_dir() || meta.uid() != me || meta.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "{} is not a private folder of this user (it must be a folder, not a link, with mode 0700)",
                    dir.display()
                ),
            ));
        }
        Ok(dir)
    }

    pub fn lock_path(data_dir: &Path) -> io::Result<PathBuf> {
        Ok(folder(data_dir)?.join("zajil.lock"))
    }

    fn socket(data_dir: &Path) -> io::Result<PathBuf> {
        let path = folder(data_dir)?.join("zajil.sock");
        if path.as_os_str().len() > MAX_SOCKET_PATH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{} is longer than the {MAX_SOCKET_PATH} bytes a socket path may have: use a shorter data folder",
                    path.display()
                ),
            ));
        }
        Ok(path)
    }

    pub struct Listener {
        listener: UnixListener,
        path: PathBuf,
    }

    impl Listener {
        /// Called with the data folder's lock held: a socket file left
        /// there is an ended app's, and is replaced.
        pub fn bind(data_dir: &Path) -> io::Result<Self> {
            let path = socket(data_dir)?;
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
            let listener = UnixListener::bind(&path)?;
            Ok(Listener { listener, path })
        }

        pub fn accept(&self) -> io::Result<Stream> {
            self.listener.accept().map(|(stream, _)| stream)
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    pub fn connect(data_dir: &Path) -> io::Result<Stream> {
        UnixStream::connect(socket(data_dir)?)
    }

    pub fn set_timeout(stream: &Stream, timeout: Duration) -> io::Result<()> {
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))
    }

    pub fn is_busy(_err: &io::Error) -> bool {
        false
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io::{self, Read};
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{
        ERROR_NO_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    pub type Stream = File;

    fn name_file(data_dir: &Path) -> PathBuf {
        data_dir.join("zajil.pipe")
    }

    pub fn lock_path(data_dir: &Path) -> io::Result<PathBuf> {
        Ok(data_dir.join("zajil.lock"))
    }

    pub struct Listener {
        name: Vec<u16>,
        file: PathBuf,
        /// The first instance is created with FILE_FLAG_FIRST_PIPE_INSTANCE:
        /// if a pipe of that name already exists, someone else made it.
        first: AtomicBool,
    }

    impl Listener {
        pub fn bind(data_dir: &Path) -> io::Result<Self> {
            let mut random = [0u8; 16];
            getrandom::fill(&mut random).map_err(io::Error::other)?;
            let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
            let name = format!(r"\\.\pipe\zajil-{hex}");
            // Readable by its owner only; replaced in one step.
            let file = name_file(data_dir);
            let tmp = data_dir.join("zajil.pipe.tmp");
            let _ = std::fs::remove_file(&tmp);
            {
                use std::io::Write;
                let mut out = crate::fs::create_private(&tmp)?;
                out.write_all(name.as_bytes())?;
                out.sync_all()?;
            }
            crate::fs::replace_file(&tmp, &file)?;
            Ok(Listener {
                name: name.encode_utf16().chain(Some(0)).collect(),
                file,
                first: AtomicBool::new(true),
            })
        }

        pub fn accept(&self) -> io::Result<Stream> {
            loop {
                // All generic access (GA) for the current user only.
                let (attributes, _descriptor) = crate::fs::owner_only("GA")?;
                let first = if self.first.swap(false, Ordering::SeqCst) {
                    FILE_FLAG_FIRST_PIPE_INSTANCE
                } else {
                    0
                };
                // SAFETY: `name` is a NUL-terminated UTF-16 string and
                // `attributes` points to a valid security descriptor, both
                // alive for the call.
                let handle = unsafe {
                    CreateNamedPipeW(
                        self.name.as_ptr(),
                        PIPE_ACCESS_DUPLEX | first,
                        PIPE_TYPE_BYTE
                            | PIPE_READMODE_BYTE
                            | PIPE_WAIT
                            | PIPE_REJECT_REMOTE_CLIENTS,
                        PIPE_UNLIMITED_INSTANCES,
                        64 * 1024,
                        64 * 1024,
                        0,
                        &attributes,
                    )
                };
                if handle == INVALID_HANDLE_VALUE {
                    return Err(io::Error::last_os_error());
                }
                // SAFETY: `handle` is a valid pipe handle that nothing else
                // owns; the `File` closes it.
                let pipe = unsafe { File::from_raw_handle(handle as RawHandle) };
                // SAFETY: `handle` is the pipe just created, used without
                // overlapped I/O (no OVERLAPPED given).
                if unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) } != 0 {
                    return Ok(pipe);
                }
                let err = io::Error::last_os_error();
                match err.raw_os_error().map(|c| c as u32) {
                    // The client connected between the two calls.
                    Some(ERROR_PIPE_CONNECTED) => return Ok(pipe),
                    // The client left before it was accepted: the next one.
                    Some(ERROR_NO_DATA) => continue,
                    _ => return Err(err),
                }
            }
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.file);
        }
    }

    pub fn connect(data_dir: &Path) -> io::Result<Stream> {
        let mut name = String::new();
        File::open(name_file(data_dir))?.read_to_string(&mut name)?;
        let name = name.trim();
        if !name.starts_with(r"\\.\pipe\zajil-") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the pipe name file of the data folder is not Zajil's",
            ));
        }
        // Identification only: the server can learn who connects, never
        // act as them. (`security_qos_flags` adds SECURITY_SQOS_PRESENT.)
        let pipe = OpenOptions::new()
            .read(true)
            .write(true)
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(name)?;
        let mut server = 0u32;
        // SAFETY: `pipe` is an open pipe handle; `server` a writable u32.
        if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle() as _, &mut server) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if !crate::fs::runs_as_current_user(server)? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the pipe named in the data folder is not this user's Zajil: its server runs as another user",
            ));
        }
        Ok(pipe)
    }

    pub fn set_timeout(_stream: &Stream, _timeout: Duration) -> io::Result<()> {
        Ok(())
    }

    pub fn is_busy(err: &io::Error) -> bool {
        err.raw_os_error() == Some(ERROR_PIPE_BUSY as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary data folder, removed afterwards.
    struct Temp(PathBuf);

    impl Temp {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "zajil-platform-ipc-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Temp(dir)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_request_and_its_answer_go_through() {
        let dir = Temp::new();
        let listener = Listener::bind(&dir.0).unwrap();
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap();
            let mut got = [0u8; 5];
            stream.read_exact(&mut got).unwrap();
            stream.write_all(b"pong").unwrap();
            got
        });
        let mut client = connect(&dir.0, Duration::from_secs(5)).unwrap();
        client.write_all(b"ping!").unwrap();
        let mut answer = [0u8; 4];
        client.read_exact(&mut answer).unwrap();
        assert_eq!(&answer, b"pong");
        assert_eq!(&server.join().unwrap(), b"ping!");
    }

    #[test]
    fn without_an_app_connecting_fails_as_not_running() {
        let dir = Temp::new();
        let err = connect(&dir.0, Duration::from_millis(200))
            .err()
            .expect("no app");
        assert!(
            matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ),
            "{err:?}"
        );
    }

    /// A second app on the same data folder is refused while the first
    /// runs, on every system; once it ends, the channel is free again, and
    /// its socket or name file is gone.
    #[test]
    fn a_second_app_is_refused_and_the_channel_is_free_after_the_first() {
        let dir = Temp::new();
        let first = Listener::bind(&dir.0).unwrap();
        let err = Listener::bind(&dir.0).err().expect("second app");
        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
        drop(first);
        assert!(!describe(&dir.0).exists(), "left behind");
        Listener::bind(&dir.0).expect("free again");
    }

    /// A socket left by an app that ended without removing it (a crash) is
    /// replaced: the lock, not the file, says who runs (Unix).
    #[cfg(unix)]
    #[test]
    fn a_crashed_apps_socket_is_replaced() {
        let dir = Temp::new();
        let first = Listener::bind(&dir.0).unwrap();
        std::mem::forget(first.inner); // the file stays, as after a crash
        drop(first._lock);
        assert!(describe(&dir.0).exists());
        let listener = Listener::bind(&dir.0).expect("replaced");
        let server = std::thread::spawn(move || listener.accept().map(|_| ()));
        connect(&dir.0, Duration::from_secs(5)).expect("reaches the new app");
        server.join().unwrap().unwrap();
    }

    /// The socket's folder is private: a folder others can enter, or a
    /// link in its place, is refused (Unix).
    #[cfg(unix)]
    #[test]
    fn the_socket_folder_must_be_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = Temp::new();
        std::fs::create_dir(dir.0.join("ipc")).unwrap();
        std::fs::set_permissions(dir.0.join("ipc"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let err = Listener::bind(&dir.0).err().expect("open folder");
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        std::fs::remove_dir(dir.0.join("ipc")).unwrap();
        std::os::unix::fs::symlink(std::env::temp_dir(), dir.0.join("ipc")).unwrap();
        let err = Listener::bind(&dir.0).err().expect("a link");
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    /// A data folder too deep for a socket path says why (Unix).
    #[cfg(unix)]
    #[test]
    fn a_socket_path_too_long_is_explained() {
        let dir = Temp::new();
        let deep = dir.0.join("a".repeat(120));
        std::fs::create_dir_all(&deep).unwrap();
        let err = Listener::bind(&deep).err().expect("too long");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("shorter data folder"), "{err}");
    }
}
