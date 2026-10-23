//! Downloading one file over HTTP or HTTPS with a single connection.
//!
//! Life cycle (docs/ARCHITECTURE.md §3.1):
//! 1. One GET request; its headers are the probe (size, `Accept-Ranges`,
//!    `ETag`, `Last-Modified`, name).
//! 2. A free name is reserved in the download folder: `<name>.zajil-part`
//!    (locked while we use it) and `<name>.zajil-state`.
//! 3. The body is written to the part file. Every `save_interval` the data is
//!    synced and then the state file records it as done.
//! 4. If the transfer stops early, the state is saved and the error says how
//!    to resume. Otherwise the part file is renamed to its final name in one
//!    step and the state file is removed.

use std::fs::TryLockError;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use reqwest::header::{self, HeaderMap};
use reqwest::{Client, Response, StatusCode, Url, redirect};
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::io::AsyncWriteExt;

use crate::naming::{ReserveError, reserve_target, suggested_name};
use crate::sanitize::ConfineError;
use crate::state::{DownloadState, StateError, Validator, part_path, state_path};

/// Engine settings. There is deliberately no setting that weakens TLS
/// certificate checks.
#[derive(Debug, Clone)]
pub struct Options {
    /// Most redirects followed for one request.
    pub max_redirects: usize,
    /// Time allowed to open a connection.
    pub connect_timeout: Duration,
    /// The transfer fails if no data arrives for this long.
    pub idle_timeout: Duration,
    /// How often progress is made durable in the state file.
    pub save_interval: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            max_redirects: 10,
            connect_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(60),
            save_interval: Duration::from_secs(1),
        }
    }
}

/// What the engine reports while it works. Interfaces show these; the engine
/// never prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Data is about to flow for `file` (written to its `.zajil-part` until
    /// it finishes), starting at byte `from`.
    Started {
        file: PathBuf,
        state_file: PathBuf,
        total: Option<u64>,
        from: u64,
    },
    /// Bytes of the file on disk so far.
    Progress { done: u64, total: Option<u64> },
    /// The download is complete at `file`.
    Finished { file: PathBuf, bytes: u64 },
}

/// The result of a complete download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub file: PathBuf,
    pub bytes: u64,
}

/// Why a download did not finish.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid URL {url:?}: {reason}")]
    InvalidUrl { url: String, reason: String },
    #[error("only http and https URLs are supported, not {0:?}")]
    UnsupportedScheme(String),
    #[error("{}", error_chain(.0))]
    Http(#[source] reqwest::Error),
    #[error("the server answered {0}")]
    Status(StatusCode),
    #[error("the download stopped ({reason}); resume it from {}", state_file.display())]
    Interrupted { state_file: PathBuf, reason: String },
    #[error("the download failed before any data arrived: {0}")]
    NoData(String),
    #[error("another process is already downloading {}", .0.display())]
    Locked(PathBuf),
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Confine(#[from] ConfineError),
    #[error(transparent)]
    Name(#[from] ReserveError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The download engine. Cheap to share; one client is reused.
#[derive(Debug, Clone)]
pub struct Engine {
    client: Client,
    options: Options,
}

impl Engine {
    pub fn new(options: Options) -> Result<Self, Error> {
        let client = build_client(&options)?;
        Ok(Engine { client, options })
    }

    /// Downloads `url` into the existing folder `dir`.
    pub async fn download(
        &self,
        url: &str,
        dir: &Path,
        on_event: &mut dyn FnMut(Event),
    ) -> Result<Finished, Error> {
        let url = parse_url(url)?;
        let response = self.get(url.clone(), None).await?;
        if response.status() != StatusCode::OK {
            return Err(Error::Status(response.status()));
        }

        let headers = response.headers();
        let content_length = response.content_length();
        let accept_ranges = header_str(headers, header::ACCEPT_RANGES).is_some_and(|v| {
            v.split(',')
                .any(|unit| unit.trim().eq_ignore_ascii_case("bytes"))
        });
        let validator = validator_from(headers);
        let name = suggested_name(
            headers
                .get(header::CONTENT_DISPOSITION)
                .map(|v| v.as_bytes()),
            response.url(),
        );

        let (final_path, part) = reserve_target(dir, &name)?;
        // From here until the first byte is written, any failure or
        // cancellation removes the new files again.
        let cleanup = EmptyDownloadCleanup::new(&final_path);
        lock(&part, &final_path)?;
        if let Some(len) = content_length {
            // Best effort reservation; sparse on some file systems.
            part.set_len(len)?;
        }
        let mut state = DownloadState::new(
            without_credentials(&url),
            without_credentials(response.url()),
            validator,
            content_length,
            accept_ranges,
        );
        let state_file = state_path(&final_path);
        save_state(&mut state, &state_file, cleanup.watch()).await?;

        let transfer = Transfer {
            file: tokio::fs::File::from_std(part),
            state,
            state_file,
            final_path,
            last_save: Instant::now(),
            save_interval: self.options.save_interval,
            cleanup: Some(cleanup),
        };
        transfer.receive(response, on_event).await
    }

    async fn get(&self, url: Url, range_from: Option<(u64, String)>) -> Result<Response, Error> {
        let mut request = self.client.get(url);
        if let Some((from, if_range)) = range_from {
            request = request
                .header(header::RANGE, format!("bytes={from}-"))
                .header(header::IF_RANGE, if_range);
        }
        request.send().await.map_err(Error::Http)
    }
}

/// An open download: the locked part file and its state.
struct Transfer {
    file: tokio::fs::File,
    state: DownloadState,
    state_file: PathBuf,
    final_path: PathBuf,
    last_save: Instant,
    save_interval: Duration,
    /// Set for a new download until its first byte is written. Declared
    /// last, so the part file is closed before it is removed.
    cleanup: Option<EmptyDownloadCleanup>,
}

/// Removes a new download's `.zajil-part` and `.zajil-state` when dropped,
/// unless disarmed. A download that fails or is cancelled before any byte
/// arrives would otherwise leave files that take the name (the next
/// download would become `name (1)`) and cannot be resumed usefully.
///
/// A state save may still be running on a blocking thread when the guard is
/// dropped (a blocking task cannot be cancelled). If the guard removed the
/// files at once, that save would write the state file afterwards, and the
/// freed name could already belong to a new download whose files the old
/// save would then touch. So the guard and every save made while it is
/// armed share one [`NewDownload`] in an `Arc`: dropping the guard only
/// marks it abandoned, and the files are removed when the **last** owner
/// lets go, after the guard and every save are done. Until then the
/// `.zajil-part` file keeps the name reserved, so a new download takes
/// `name (1)` instead. Nothing waits on the async thread.
struct EmptyDownloadCleanup {
    new: Arc<NewDownload>,
    armed: bool,
}

/// What the guard and the saves share.
struct NewDownload {
    final_path: PathBuf,
    abandoned: AtomicBool,
}

impl Drop for NewDownload {
    /// Runs once the guard and every save are finished. The state file goes
    /// first and the part file last, so the name stays reserved (by the part
    /// file) until nothing of this download is left.
    fn drop(&mut self) {
        if self.abandoned.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(state_path(&self.final_path));
            let _ = std::fs::remove_file(part_path(&self.final_path));
        }
    }
}

impl EmptyDownloadCleanup {
    fn new(final_path: &Path) -> Self {
        EmptyDownloadCleanup {
            new: Arc::new(NewDownload {
                final_path: final_path.to_path_buf(),
                abandoned: AtomicBool::new(false),
            }),
            armed: true,
        }
    }

    /// For a save made while the guard is armed.
    fn watch(&self) -> Option<Arc<NewDownload>> {
        self.armed.then(|| self.new.clone())
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for EmptyDownloadCleanup {
    fn drop(&mut self) {
        if self.armed {
            // The files go when the last owner of `new` is dropped: here,
            // or at the end of a save that is still running.
            self.new.abandoned.store(true, Ordering::SeqCst);
        }
    }
}

impl Transfer {
    fn done(&self) -> u64 {
        self.state.segments[0].done
    }

    /// Writes the body after the current position, then finishes.
    async fn receive(
        mut self,
        mut response: Response,
        on_event: &mut dyn FnMut(Event),
    ) -> Result<Finished, Error> {
        let total = self.state.content_length;
        on_event(Event::Started {
            file: self.final_path.clone(),
            state_file: self.state_file.clone(),
            total,
            from: self.done(),
        });

        loop {
            let chunk = match response.chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(err) => return Err(self.interrupt(error_chain(&err)).await),
            };
            if total.is_some_and(|t| self.done() + chunk.len() as u64 > t) {
                return Err(self
                    .interrupt("the server sent more data than it announced".into())
                    .await);
            }
            if let Err(err) = self.file.write_all(&chunk).await {
                return Err(self.interrupt(err.to_string()).await);
            }
            if let Some(cleanup) = self.cleanup.take() {
                cleanup.disarm();
            }
            self.state.segments[0].done += chunk.len() as u64;
            on_event(Event::Progress {
                done: self.done(),
                total,
            });
            if self.last_save.elapsed() >= self.save_interval
                && let Err(err) = self.checkpoint().await
            {
                return Err(self.interrupt(err.to_string()).await);
            }
        }

        if let Some(total) = total
            && self.done() < total
        {
            let reason = format!(
                "the connection closed after {} of {total} bytes",
                self.done()
            );
            return Err(self.interrupt(reason).await);
        }
        self.finish(on_event).await
    }

    /// Syncs the data, then records it in the state file (ADR-0004 rule 1).
    async fn checkpoint(&mut self) -> Result<(), Error> {
        self.file.flush().await?;
        self.file.sync_data().await?;
        let watch = self.cleanup.as_ref().and_then(EmptyDownloadCleanup::watch);
        save_state(&mut self.state, &self.state_file, watch).await?;
        self.last_save = Instant::now();
        Ok(())
    }

    /// Saves what we have and turns `reason` into a resumable error. A new
    /// download that has nothing written yet is not resumable: its files are
    /// removed (when `self` is dropped) and the error says so.
    async fn interrupt(mut self, reason: String) -> Error {
        if self.cleanup.is_some() {
            return Error::NoData(reason);
        }
        if let Err(err) = self.checkpoint().await {
            return err;
        }
        Error::Interrupted {
            state_file: self.state_file,
            reason,
        }
    }

    async fn finish(mut self, on_event: &mut dyn FnMut(Event)) -> Result<Finished, Error> {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup.disarm(); // an empty file is a complete download
        }
        let bytes = self.done();
        self.file.flush().await?;
        // A file of unknown size may have been pre-sized differently.
        self.file.set_len(bytes).await?;
        self.file.sync_all().await?;
        // Record that everything is on disk before renaming, so a failed
        // rename leaves a download that `resume` completes at once.
        save_state(&mut self.state, &self.state_file, None).await?;
        drop(self.file); // releases the lock before the rename

        let part = part_path(&self.final_path);
        let wanted = self.final_path.clone();
        let placed = blocking(move || place_final(&part, &wanted)).await;
        let file = match placed {
            Ok(file) => file,
            Err(err) => {
                return Err(Error::Interrupted {
                    state_file: self.state_file,
                    reason: format!("could not give the finished file its name: {err}"),
                });
            }
        };
        // The download is complete. A state file that cannot be removed is
        // left behind rather than reported as a failed download.
        let _ = std::fs::remove_file(&self.state_file);
        let dir = parent_dir(&file).to_path_buf();
        let _ = blocking(move || zajil_platform::fs::sync_dir(&dir)).await;
        on_event(Event::Finished {
            file: file.clone(),
            bytes,
        });
        Ok(Finished { file, bytes })
    }
}

fn build_client(options: &Options) -> Result<Client, Error> {
    // rustls with the `ring` provider and the operating system's certificate
    // store and checks. Nothing here can turn verification off.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .and_then(|builder| builder.with_platform_verifier())
        .map_err(|err| io::Error::other(format!("TLS setup failed: {err}")))?
        .with_no_client_auth();

    let max_redirects = options.max_redirects;
    let policy = redirect::Policy::custom(move |attempt| {
        let downgrade = attempt.url().scheme() != "https"
            && attempt.previous().iter().any(|url| url.scheme() == "https");
        if attempt.previous().len() > max_redirects {
            attempt.error(format!("more than {max_redirects} redirects"))
        } else if !matches!(attempt.url().scheme(), "http" | "https") {
            attempt.error("redirect to an unsupported scheme")
        } else if downgrade {
            attempt.error("refusing a redirect from https to http")
        } else {
            attempt.follow()
        }
    });

    Client::builder()
        .tls_backend_preconfigured(tls)
        .redirect(policy)
        .connect_timeout(options.connect_timeout)
        .read_timeout(options.idle_timeout)
        // Proxies arrive in M3; until then never pick them up implicitly.
        .no_proxy()
        .user_agent(concat!("Zajil/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(Error::Http)
}

fn parse_url(raw: &str) -> Result<Url, Error> {
    let url = Url::parse(raw).map_err(|err| Error::InvalidUrl {
        url: raw.to_owned(),
        reason: err.to_string(),
    })?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        other => Err(Error::UnsupportedScheme(other.to_owned())),
    }
}

fn lock(file: &std::fs::File, final_path: &Path) -> Result<(), Error> {
    match file.try_lock() {
        Ok(()) => Ok(()),
        Err(TryLockError::WouldBlock) => Err(Error::Locked(final_path.to_path_buf())),
        Err(TryLockError::Error(err)) => Err(err.into()),
    }
}

fn header_str(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn validator_from(headers: &HeaderMap) -> Validator {
    Validator {
        etag: header_str(headers, header::ETAG).map(str::to_owned),
        last_modified: header_str(headers, header::LAST_MODIFIED).map(str::to_owned),
        date: header_str(headers, header::DATE).map(str::to_owned),
    }
}

/// Saves the state file on a blocking thread: on Windows a save may wait up
/// to about a second for another program to release the file, and that wait
/// must not stop the async runtime. `new` is set for a new download that has
/// no data yet (see [`EmptyDownloadCleanup`]).
async fn save_state(
    state: &mut DownloadState,
    path: &Path,
    new: Option<Arc<NewDownload>>,
) -> Result<(), Error> {
    let mut copy = state.clone();
    let path = path.to_path_buf();
    *state = blocking(move || {
        let abandoned = || {
            new.as_ref()
                .is_some_and(|new| new.abandoned.load(Ordering::SeqCst))
        };
        if abandoned() {
            return Err(StateError::Io(io::Error::other(
                "the download was abandoned",
            )));
        }
        // If the download is abandoned while this runs, `new` is the last
        // owner when this closure ends, and its drop removes what was written.
        copy.save(&path).map(|()| copy)
    })
    .await?;
    Ok(())
}

/// Runs blocking file-system work off the async threads.
async fn blocking<T: Send + 'static, E: From<io::Error> + Send + 'static>(
    work: impl FnOnce() -> Result<T, E> + Send + 'static,
) -> Result<T, E> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| E::from(io::Error::other(err)))?
}

/// Renames the finished `part` to `wanted`, or to `name (1).ext` and so on
/// if that name is taken, never replacing an existing file. Another
/// download's part or state file also counts as taken.
fn place_final(part: &Path, wanted: &Path) -> io::Result<PathBuf> {
    let name = wanted
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned();
    for candidate in crate::naming::candidates(&name) {
        let target = wanted.with_file_name(candidate);
        let busy = target != wanted
            && (part_path(&target).symlink_metadata().is_ok()
                || state_path(&target).symlink_metadata().is_ok());
        if busy {
            continue;
        }
        match zajil_platform::fs::rename_noreplace(part, &target) {
            Ok(()) => return Ok(target),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::other(format!("no free file name for {name:?}")))
}

/// The folder of `path`, or `.` for a bare file name.
fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    }
}

/// `url` without a user name or password. Credentials in a URL are used for
/// the request but never written to the state file (ADR-0004, rule 7).
fn without_credentials(url: &Url) -> String {
    let mut url = url.clone();
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.to_string()
}

/// An error and all its sources, so TLS and connection details reach the user.
fn error_chain(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}
