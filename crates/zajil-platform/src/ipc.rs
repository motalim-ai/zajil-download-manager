//! A channel between Zajil's own programs on one computer, for the current
//! user only (ADR-0005): `zajil-host`, which the browser starts for the
//! extension, hands each request to the running app through it.
//!
//! | | Where | Who can connect |
//! |---|---|---|
//! | macOS, Linux | a Unix socket `<data dir>/ipc/zajil.sock` | the folder is `0700`, so only its owner reaches the socket, from the moment it exists |
//! | Windows | a named pipe with a random name, written in `<data dir>/zajil.pipe` | the pipe's access list allows the current user only, remote clients are refused, and the name file is readable by its owner only |
//!
//! Pipe names on Windows are global to the computer: another user could
//! create a pipe of a name known in advance before Zajil does, and receive
//! what was meant for it. So the name is random for each run of the app,
//! and only the owner of the data folder can read it.
//!
//! Each connection carries one request and its answer; the format is the
//! caller's (`zajil-host`'s protocol).

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A connection on the channel, from either side.
pub struct Stream(imp::Stream);

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

/// The app's end: waits for connections.
pub struct Listener(imp::Listener);

impl Listener {
    /// Opens the channel of the data folder `data_dir`. Fails with
    /// [`io::ErrorKind::AddrInUse`] while another program has it open (a
    /// second app on the same data folder).
    pub fn bind(data_dir: &Path) -> io::Result<Self> {
        imp::Listener::bind(data_dir).map(Listener)
    }

    /// Waits for the next connection.
    pub fn accept(&self) -> io::Result<Stream> {
        self.0.accept().map(Stream)
    }
}

/// Connects to the app of the data folder `data_dir`, waiting up to
/// `patience` while it is starting. [`io::ErrorKind::NotFound`] or
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

#[cfg(unix)]
mod imp {
    use std::fs::DirBuilder;
    use std::io;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};

    pub type Stream = UnixStream;

    /// `<data dir>/ipc`: made `0700`, or checked to be a folder of ours
    /// that only we can enter (never a link).
    fn folder(data_dir: &Path) -> io::Result<PathBuf> {
        let dir = data_dir.join("ipc");
        match DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
        let meta = std::fs::symlink_metadata(&dir)?;
        // The owner of the data folder: this user.
        let ours = std::fs::metadata(data_dir)?.uid();
        if !meta.is_dir() || meta.uid() != ours || meta.permissions().mode() & 0o077 != 0 {
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

    fn socket(data_dir: &Path) -> io::Result<PathBuf> {
        Ok(folder(data_dir)?.join("zajil.sock"))
    }

    pub struct Listener(UnixListener);

    impl Listener {
        pub fn bind(data_dir: &Path) -> io::Result<Self> {
            let path = socket(data_dir)?;
            match UnixListener::bind(&path) {
                Ok(listener) => Ok(Listener(listener)),
                Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
                    // Left by an app that ended without removing it, or
                    // open in another app on this data folder.
                    if UnixStream::connect(&path).is_ok() {
                        return Err(io::Error::new(
                            io::ErrorKind::AddrInUse,
                            "another Zajil is running on this data folder",
                        ));
                    }
                    std::fs::remove_file(&path)?;
                    UnixListener::bind(&path).map(Listener)
                }
                Err(err) => Err(err),
            }
        }

        pub fn accept(&self) -> io::Result<Stream> {
            self.0.accept().map(|(stream, _)| stream)
        }
    }

    pub fn connect(data_dir: &Path) -> io::Result<Stream> {
        UnixStream::connect(socket(data_dir)?)
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
    use std::os::windows::io::{FromRawHandle, RawHandle};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};

    use windows_sys::Win32::Foundation::{
        ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    pub type Stream = File;

    fn name_file(data_dir: &Path) -> PathBuf {
        data_dir.join("zajil.pipe")
    }

    pub struct Listener {
        name: Vec<u16>,
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
            let listener = Listener {
                name: name.encode_utf16().chain(Some(0)).collect(),
                first: AtomicBool::new(true),
            };
            Ok(listener)
        }

        pub fn accept(&self) -> io::Result<Stream> {
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
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
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
            let ok = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
            if ok == 0 {
                let err = io::Error::last_os_error();
                // The client connected between the two calls.
                if err.raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32) {
                    return Err(err);
                }
            }
            Ok(pipe)
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
        OpenOptions::new().read(true).write(true).open(name)
    }

    pub fn is_busy(err: &io::Error) -> bool {
        err.raw_os_error() == Some(ERROR_PIPE_BUSY as i32)
    }
}

/// Where the channel of `data_dir` is found (for messages).
pub fn describe(data_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        data_dir.join("zajil.pipe")
    } else {
        data_dir.join("ipc").join("zajil.sock")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "zajil-platform-ipc-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_request_and_its_answer_go_through() {
        let dir = tempdir();
        let listener = Listener::bind(&dir).unwrap();
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap();
            let mut got = [0u8; 5];
            stream.read_exact(&mut got).unwrap();
            stream.write_all(b"pong").unwrap();
            got
        });
        let mut client = connect(&dir, Duration::from_secs(5)).unwrap();
        client.write_all(b"ping!").unwrap();
        let mut answer = [0u8; 4];
        client.read_exact(&mut answer).unwrap();
        assert_eq!(&answer, b"pong");
        assert_eq!(&server.join().unwrap(), b"ping!");
    }

    #[test]
    fn without_an_app_connecting_fails_as_not_running() {
        let dir = tempdir();
        let err = connect(&dir, Duration::from_millis(200))
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

    /// A second app on the same data folder is told so; a socket left by
    /// an app that ended is taken over (Unix).
    #[cfg(unix)]
    #[test]
    fn a_second_app_on_the_same_folder_is_refused_and_a_stale_socket_reused() {
        let dir = tempdir();
        let first = Listener::bind(&dir).unwrap();
        let err = Listener::bind(&dir).err().expect("second app");
        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
        drop(first);
        // The socket file is still there, with nobody behind it.
        assert!(describe(&dir).exists());
        Listener::bind(&dir).expect("taken over");
    }

    /// The socket's folder is private: a folder others can enter, or a
    /// link in its place, is refused (Unix).
    #[cfg(unix)]
    #[test]
    fn the_socket_folder_must_be_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir();
        std::fs::create_dir(dir.join("ipc")).unwrap();
        std::fs::set_permissions(dir.join("ipc"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = Listener::bind(&dir).err().expect("open folder");
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        std::fs::remove_dir(dir.join("ipc")).unwrap();
        std::os::unix::fs::symlink(std::env::temp_dir(), dir.join("ipc")).unwrap();
        let err = Listener::bind(&dir).err().expect("a link");
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }
}
