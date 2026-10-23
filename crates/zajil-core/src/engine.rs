//! Downloading one file over HTTP or HTTPS.
//!
//! Life cycle (docs/ARCHITECTURE.md §3.1):
//! 1. One GET request; its headers are the probe (size, `Accept-Ranges`,
//!    `ETag`, `Last-Modified`, name).
//! 2. If the size is known, it must fit in the free space of the folder
//!    before any file is created.
//! 3. A free name is reserved in the download folder: `<name>.zajil-part`
//!    (locked while we use it) and `<name>.zajil-state`, and the part file is
//!    given its size (`zajil_platform::fs::allocate`).
//! 4. A server that announces ranges, the size and a strong validator gets
//!    several connections (`segmented`); otherwise the body of the probe is
//!    written on one connection. Every `save_interval` the data is synced and
//!    then the state file records it as done.
//! 5. If the transfer stops early, the state is saved and the error says how
//!    to resume. Otherwise the part file is renamed to its final name in one
//!    step and the state file is removed.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use reqwest::header::{self, HeaderMap};
use reqwest::{Client, Response, StatusCode, Url, redirect};
use tokio::io::AsyncWriteExt;

use crate::http_date;
use crate::naming::{ReserveError, reserve_target, suggested_name};
use crate::sanitize::{ConfineError, MAX_NAME_BYTES, confine, sanitize_file_name};
use crate::segmented::{self, Job, Outcome};
use crate::state::{
    DownloadState, StateError, Validator, final_path_for_state, part_path, state_path,
};

/// How many times one call may start over because the file changed on the
/// server, before giving up for now.
const MAX_RESTARTS: u8 = 3;

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
    /// Connections for one download, from 1 to [`crate::MAX_CONNECTIONS`] (other
    /// values are clamped). A file is split only if its server announces
    /// `Accept-Ranges: bytes`, its size and a strong validator.
    pub connections: usize,
    /// No segment is made smaller than this many bytes.
    pub min_segment_size: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            max_redirects: 10,
            connect_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(60),
            save_interval: Duration::from_secs(1),
            connections: crate::DEFAULT_CONNECTIONS,
            min_segment_size: 1024 * 1024,
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
    /// A resumed download had to start again from byte 0.
    Restarted { reason: RestartReason },
    /// The download is complete at `file`.
    Finished { file: PathBuf, bytes: u64 },
}

/// Why a resumed download started again from the beginning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartReason {
    /// The file on the server is a different version (new `ETag`,
    /// `Last-Modified` or size).
    FileChanged,
    /// The server answered a range request with the whole file.
    RangesIgnored,
    /// The partial range did not continue where our data ends.
    RangeMismatch,
    /// The server gave no strong validator (a strong `ETag`, or a
    /// `Last-Modified` at least one second before its `Date`), so a change
    /// could not be detected safely (RFC 9110 §13.1.5).
    NoValidator,
    /// No data had been written yet.
    NothingDone,
    /// The `.zajil-part` file was missing.
    PartFileMissing,
    /// The `.zajil-part` file is shorter than the state says.
    PartFileTooShort,
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
    #[error("refusing to resume {name:?}: {reason}")]
    UnsafeName { name: String, reason: String },
    #[error("the download failed before any data arrived: {0}")]
    NoData(String),
    #[error(
        "not enough free space in {}: the file needs {needed} bytes and {available} are free",
        dir.display()
    )]
    NotEnoughSpace {
        dir: PathBuf,
        needed: u64,
        available: u64,
    },
    #[error("could not make room for {bytes} bytes in {}: {source}", file.display())]
    Allocate {
        file: PathBuf,
        bytes: u64,
        #[source]
        source: io::Error,
    },
    #[error("another process is already downloading {}", .0.display())]
    Locked(PathBuf),
    #[error(
        "refusing to use {}: it is a symbolic link, a hard link or not a regular file",
        .0.display()
    )]
    NotRegularFile(PathBuf),
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
        let client = build_client(&options, &[])?;
        Ok(Engine { client, options })
    }

    /// **Test builds only.** Like [`Engine::new`], but also trusts the
    /// certificate authority `ca_der` (DER), next to the system store.
    ///
    /// Compiled only with the `test-support` feature, which only
    /// `dev-dependencies` enable: the shipped `zajil` has no way to add a
    /// trusted root, and CI checks that its release build lacks the feature.
    #[cfg(feature = "test-support")]
    pub fn with_test_root(options: Options, ca_der: &[u8]) -> Result<Self, Error> {
        let client = build_client(&options, &[ca_der.to_vec()])?;
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
        // The size comes from the server: before creating anything, make
        // sure it fits (a huge or false size fails here, clearly).
        if let Some(len) = response.content_length() {
            check_space(dir, len, 0).await?;
        }

        let name = suggested_name(
            response
                .headers()
                .get(header::CONTENT_DISPOSITION)
                .map(|v| v.as_bytes()),
            response.url(),
        );
        let (final_path, part) = reserve_target(dir, &name)?;
        // From here until the first byte is written, any failure or
        // cancellation removes the new files again.
        let cleanup = EmptyDownloadCleanup::new(&final_path);
        lock(&part, &final_path)?;
        let state = state_from_response(without_credentials(&url), &response);
        self.start(
            final_path,
            part,
            state,
            response,
            Some(cleanup),
            MAX_RESTARTS,
            on_event,
        )
        .await
    }

    /// Continues the download described by `state_file`
    /// (`<folder>/<name>.zajil-state`).
    ///
    /// Asks only for the bytes that are not on disk yet, with `Range` and
    /// `If-Range` (RFC 9110 §14.2, §13.1.5). If the server sends the whole
    /// file instead (it changed, or it ignores ranges), the download starts
    /// over from that response. Old and new content are never mixed.
    pub async fn resume(
        &self,
        state_file: &Path,
        on_event: &mut dyn FnMut(Event),
    ) -> Result<Finished, Error> {
        // The state file and everything next to it are untrusted: they may
        // come from another machine. Work with an absolute path inside the
        // real folder, with a name that sanitizing leaves unchanged, and
        // never follow a link (`.zajil-state`, `.zajil-part` and the
        // temporary file are all opened without following links).
        let state_file = std::path::absolute(state_file)?;
        let final_path =
            final_path_for_state(&state_file).ok_or(Error::State(StateError::NotAStateFile))?;
        let name = final_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let sanitized = sanitize_file_name(name);
        if sanitized != name {
            let reason = if name.len() > MAX_NAME_BYTES {
                format!(
                    "the name is {} bytes long, more than the {MAX_NAME_BYTES}-byte limit",
                    name.len()
                )
            } else if sanitized == format!("_{name}") {
                // Windows reads the part before the first dot, without
                // trailing spaces, as the device name.
                let device = name.split('.').next().unwrap_or(name).trim_end();
                format!(
                    "`{device}` is a device name Windows reserves, so Zajil never uses it for a file"
                )
            } else {
                format!("it is not a name Zajil would create (Zajil would write {sanitized:?})")
            };
            return Err(Error::UnsafeName {
                name: name.to_owned(),
                reason,
            });
        }
        let final_path = confine(parent_dir(&final_path), name)?;
        let state_file = state_path(&final_path);
        let state = DownloadState::load(&state_file)?;
        let part_file = part_path(&final_path);
        let part = match zajil_platform::fs::open_regular_nofollow(&part_file, true) {
            Ok(part) => Some(part),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) if zajil_platform::fs::is_not_regular_file(&err) => {
                return Err(Error::NotRegularFile(part_file));
            }
            Err(err) => return Err(err.into()),
        };

        let Some(part) = part else {
            // Nothing on disk to continue from.
            let part = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&part_file)?;
            lock(&part, &final_path)?;
            return self
                .restart(
                    final_path,
                    part,
                    state,
                    RestartReason::PartFileMissing,
                    None,
                    MAX_RESTARTS,
                    on_event,
                )
                .await;
        };
        lock(&part, &final_path)?;

        let done = state.bytes_done();
        // The data must reach the last byte the state counts as written.
        let written_end = state
            .segments
            .iter()
            .map(|s| s.start.saturating_add(s.done))
            .max()
            .unwrap_or(0);
        let on_disk = part.metadata()?.len();
        let if_range = if_range_value(&state.validator);
        let reason = if on_disk < written_end {
            Some(RestartReason::PartFileTooShort)
        } else if done == 0 {
            Some(RestartReason::NothingDone)
        } else if if_range.is_none() {
            // Without a validator a changed file could not be detected.
            Some(RestartReason::NoValidator)
        } else {
            None
        };
        if let Some(reason) = reason {
            return self
                .restart(
                    final_path,
                    part,
                    state,
                    reason,
                    None,
                    MAX_RESTARTS,
                    on_event,
                )
                .await;
        }
        let if_range = if_range.expect("checked above");
        if state.segments.len() > 1 {
            return self
                .resume_segments(final_path, part, state, if_range, on_event)
                .await;
        }

        let response = self
            .get_with_fallback(&state, Some((done, None, if_range)))
            .await?;
        match response.status() {
            StatusCode::PARTIAL_CONTENT => {
                if !continues_at(&response, &state, done) {
                    return self
                        .restart(
                            final_path,
                            part,
                            state,
                            RestartReason::RangeMismatch,
                            None,
                            MAX_RESTARTS,
                            on_event,
                        )
                        .await;
                }
                let mut part = part;
                part.seek(SeekFrom::Start(done))?;
                self.transfer(final_path, part, state, None)
                    .receive(response, on_event)
                    .await
            }
            StatusCode::OK => {
                let reason = if changed(&state, &response) {
                    RestartReason::FileChanged
                } else {
                    RestartReason::RangesIgnored
                };
                on_event(Event::Restarted { reason });
                let state = state_from_response(state.url.clone(), &response);
                self.start(
                    final_path,
                    part,
                    state,
                    response,
                    None,
                    MAX_RESTARTS,
                    on_event,
                )
                .await
            }
            StatusCode::RANGE_NOT_SATISFIABLE if state.content_length == Some(done) => {
                // Everything was already on disk; only the rename was missing.
                self.transfer(final_path, part, state, None)
                    .finish(on_event)
                    .await
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                self.restart(
                    final_path,
                    part,
                    state,
                    RestartReason::RangeMismatch,
                    None,
                    MAX_RESTARTS,
                    on_event,
                )
                .await
            }
            status => Err(Error::Status(status)),
        }
    }

    /// Resumes a download made of several segments: the first missing range
    /// is checked like a single-connection resume (with a fallback to the
    /// original URL), then every missing range gets a connection.
    async fn resume_segments(
        &self,
        final_path: PathBuf,
        part: File,
        state: DownloadState,
        if_range: String,
        on_event: &mut dyn FnMut(Event),
    ) -> Result<Finished, Error> {
        let missing = state
            .segments
            .iter()
            .find(|s| s.end.is_some_and(|end| s.start + s.done <= end))
            .cloned();
        let Some(segment) = missing else {
            // Every byte is on disk; only the rename was missing.
            return finish_download(part, state, final_path, on_event).await;
        };
        let (from, to) = (segment.start + segment.done, segment.end.expect("checked"));
        let response = self
            .get_with_fallback(&state, Some((from, Some(to), if_range.clone())))
            .await?;
        match response.status() {
            StatusCode::PARTIAL_CONTENT => {
                let range = header_str(response.headers(), header::CONTENT_RANGE)
                    .and_then(parse_content_range);
                let exact = range.is_some_and(|r| {
                    r.start == from && r.end == to && r.total == state.content_length
                });
                if !exact || changed(&state, &response) {
                    let reason = if exact {
                        RestartReason::FileChanged
                    } else {
                        RestartReason::RangeMismatch
                    };
                    return self
                        .restart(
                            final_path,
                            part,
                            state,
                            reason,
                            None,
                            MAX_RESTARTS,
                            on_event,
                        )
                        .await;
                }
                let job = Job {
                    client: self.client.clone(),
                    connections: self.options.connections,
                    save_interval: self.options.save_interval,
                    url: response.url().clone(),
                    if_range,
                    final_path,
                    part,
                    state,
                    first: Some((response, segment.start)),
                    cleanup: None,
                };
                self.run_segments(job, MAX_RESTARTS, on_event).await
            }
            StatusCode::OK => {
                let reason = if changed(&state, &response) {
                    RestartReason::FileChanged
                } else {
                    RestartReason::RangesIgnored
                };
                on_event(Event::Restarted { reason });
                let state = state_from_response(state.url.clone(), &response);
                self.start(
                    final_path,
                    part,
                    state,
                    response,
                    None,
                    MAX_RESTARTS,
                    on_event,
                )
                .await
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                self.restart(
                    final_path,
                    part,
                    state,
                    RestartReason::RangeMismatch,
                    None,
                    MAX_RESTARTS,
                    on_event,
                )
                .await
            }
            status => Err(Error::Status(status)),
        }
    }

    /// Runs a segmented download, and starts over if the file changed.
    async fn run_segments(
        &self,
        job: Job,
        restarts_left: u8,
        on_event: &mut dyn FnMut(Event),
    ) -> Result<Finished, Error> {
        let final_path = job.final_path.clone();
        let old = job.state.clone();
        match segmented::run(job, on_event).await? {
            Outcome::Finished(finished) => Ok(finished),
            Outcome::Restart {
                part,
                reason,
                response,
                cleanup,
            } => match response {
                Some(response) => {
                    on_event(Event::Restarted { reason });
                    let state = state_from_response(old.url, &response);
                    Box::pin(self.start(
                        final_path,
                        part,
                        state,
                        response,
                        cleanup,
                        restarts_left,
                        on_event,
                    ))
                    .await
                }
                None => {
                    Box::pin(self.restart(
                        final_path,
                        part,
                        old,
                        reason,
                        cleanup,
                        restarts_left,
                        on_event,
                    ))
                    .await
                }
            },
        }
    }

    /// Starts over with a plain GET, reporting `reason`.
    #[allow(clippy::too_many_arguments)]
    async fn restart(
        &self,
        final_path: PathBuf,
        part: File,
        old: DownloadState,
        reason: RestartReason,
        cleanup: Option<EmptyDownloadCleanup>,
        restarts_left: u8,
        on_event: &mut dyn FnMut(Event),
    ) -> Result<Finished, Error> {
        on_event(Event::Restarted { reason });
        let response = self.get_with_fallback(&old, None).await?;
        if response.status() != StatusCode::OK {
            return Err(Error::Status(response.status()));
        }
        let state = state_from_response(old.url, &response);
        self.start(
            final_path,
            part,
            state,
            response,
            cleanup,
            restarts_left,
            on_event,
        )
        .await
    }

    /// Writes a whole-file `200` response into `part` from byte 0, over
    /// several connections when the server allows it. `cleanup` is set for a
    /// new download (see [`EmptyDownloadCleanup`]).
    #[allow(clippy::too_many_arguments)]
    async fn start(
        &self,
        final_path: PathBuf,
        part: File,
        mut state: DownloadState,
        response: Response,
        cleanup: Option<EmptyDownloadCleanup>,
        restarts_left: u8,
        on_event: &mut dyn FnMut(Event),
    ) -> Result<Finished, Error> {
        let Some(restarts_left) = restarts_left.checked_sub(1) else {
            return Err(Error::Interrupted {
                state_file: state_path(&final_path),
                reason: "the file kept changing on the server; try again later".into(),
            });
        };
        let len = state.content_length;
        let part_file = part_path(&final_path);
        // Like a save, the allocation keeps a cancelled new download's name
        // reserved until it is over (see `EmptyDownloadCleanup`). It hands
        // the guard's share back next to the file, so if nobody waits for
        // the result any more, the file is closed before the name is freed.
        let keep_alive = cleanup.as_ref().and_then(EmptyDownloadCleanup::watch);
        let (part, keep_alive) = blocking(move || {
            let mut part = part;
            part.set_len(0)?;
            if let Some(len) = len {
                zajil_platform::fs::allocate(&part, len).map_err(|source| Error::Allocate {
                    file: part_file,
                    bytes: len,
                    source,
                })?;
            }
            part.seek(SeekFrom::Start(0))?;
            Ok::<_, Error>((part, keep_alive))
        })
        .await?;
        drop(keep_alive);

        let split = self.split(&state);
        let watch = cleanup.as_ref().and_then(EmptyDownloadCleanup::watch);
        if let Some(segments) = split {
            state.segments = segments;
            save_state(&mut state, &state_path(&final_path), watch).await?;
            let if_range = if_range_value(&state.validator).expect("checked by split");
            let job = Job {
                client: self.client.clone(),
                connections: self.options.connections,
                save_interval: self.options.save_interval,
                url: response.url().clone(),
                if_range,
                final_path,
                part,
                state,
                first: Some((response, 0)),
                cleanup,
            };
            return self.run_segments(job, restarts_left, on_event).await;
        }
        save_state(&mut state, &state_path(&final_path), watch).await?;
        self.transfer(final_path, part, state, cleanup)
            .receive(response, on_event)
            .await
    }

    /// The segments for a parallel download, or `None` for one connection:
    /// the server must announce ranges, the size and a strong validator.
    fn split(&self, state: &DownloadState) -> Option<Vec<crate::state::Segment>> {
        let len = state.content_length?;
        if !state.accept_ranges || if_range_value(&state.validator).is_none() {
            return None;
        }
        segmented::plan(len, self.options.connections, self.options.min_segment_size)
    }

    fn transfer(
        &self,
        final_path: PathBuf,
        part: File,
        state: DownloadState,
        cleanup: Option<EmptyDownloadCleanup>,
    ) -> Transfer {
        Transfer {
            file: tokio::fs::File::from_std(part),
            state,
            state_file: state_path(&final_path),
            final_path,
            last_save: Instant::now(),
            save_interval: self.options.save_interval,
            cleanup,
        }
    }

    /// Requests `final_url`, and the original URL if that fails: signed CDN
    /// links often expire, while the original URL redirects to a fresh one.
    async fn get_with_fallback(
        &self,
        state: &DownloadState,
        range: Option<(u64, Option<u64>, String)>,
    ) -> Result<Response, Error> {
        let final_url = parse_url(&state.final_url)?;
        let first = self.get(final_url, range.clone()).await;
        let usable = |r: &Result<Response, Error>| {
            r.as_ref().is_ok_and(|r| {
                matches!(
                    r.status(),
                    StatusCode::OK
                        | StatusCode::PARTIAL_CONTENT
                        | StatusCode::RANGE_NOT_SATISFIABLE
                )
            })
        };
        if usable(&first) || state.final_url == state.url {
            return first;
        }
        self.get(parse_url(&state.url)?, range).await
    }

    /// GETs `url`; with `range`, only bytes `from` to `to` (or to the end)
    /// and only if the file still matches `If-Range`.
    async fn get(
        &self,
        url: Url,
        range: Option<(u64, Option<u64>, String)>,
    ) -> Result<Response, Error> {
        let mut request = self.client.get(url);
        if let Some((from, to, if_range)) = range {
            let to = to.map(|to| to.to_string()).unwrap_or_default();
            request = request
                .header(header::RANGE, format!("bytes={from}-{to}"))
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
pub(crate) struct EmptyDownloadCleanup {
    new: Arc<NewDownload>,
    armed: bool,
}

/// What the guard and the saves share (and, for a segmented download, the
/// writes).
pub(crate) struct NewDownload {
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
    pub(crate) fn watch(&self) -> Option<Arc<NewDownload>> {
        self.armed.then(|| self.new.clone())
    }

    pub(crate) fn disarm(mut self) {
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
        self.file.flush().await?;
        let file = self.file.into_std().await;
        finish_download(file, self.state, self.final_path, on_event).await
    }
}

/// Completes a download whose bytes are all written to `part`: syncs it,
/// records that everything is on disk, renames it to its final name (or a
/// numbered one) and removes the state file.
pub(crate) async fn finish_download(
    part: File,
    mut state: DownloadState,
    final_path: PathBuf,
    on_event: &mut dyn FnMut(Event),
) -> Result<Finished, Error> {
    let bytes = state.bytes_done();
    let state_file = state_path(&final_path);
    let part = blocking(move || {
        // A file of unknown size may have been pre-sized differently.
        part.set_len(bytes)?;
        part.sync_all()?;
        Ok::<_, io::Error>(part)
    })
    .await?;
    // Record that everything is on disk before renaming, so a failed
    // rename leaves a download that `resume` completes at once.
    save_state(&mut state, &state_file, None).await?;
    drop(part); // releases the lock before the rename

    let part = part_path(&final_path);
    let wanted = final_path.clone();
    let placed = blocking(move || place_final(&part, &wanted)).await;
    let file = match placed {
        Ok(file) => file,
        Err(err) => {
            return Err(Error::Interrupted {
                state_file,
                reason: format!("could not give the finished file its name: {err}"),
            });
        }
    };
    // The download is complete. A state file that cannot be removed is
    // left behind rather than reported as a failed download.
    let _ = std::fs::remove_file(&state_file);
    let dir = parent_dir(&file).to_path_buf();
    let _ = blocking(move || zajil_platform::fs::sync_dir(&dir)).await;
    on_event(Event::Finished {
        file: file.clone(),
        bytes,
    });
    Ok(Finished { file, bytes })
}

/// Fails with [`Error::NotEnoughSpace`] unless `needed` bytes fit in the free
/// space of `dir`, counting `reusable` bytes already set aside there.
async fn check_space(dir: &Path, needed: u64, reusable: u64) -> Result<(), Error> {
    let folder = dir.to_path_buf();
    let available = on_own_thread(move || zajil_platform::fs::available_space(&folder)).await??;
    if needed > available.saturating_add(reusable) {
        return Err(Error::NotEnoughSpace {
            dir: dir.to_path_buf(),
            needed,
            available,
        });
    }
    Ok(())
}

fn build_client(options: &Options, extra_roots: &[Vec<u8>]) -> Result<Client, Error> {
    // rustls with the `ring` provider, and reqwest's built-in use of the
    // operating system's certificate store and checks
    // (`rustls-platform-verifier`). Nothing here can turn verification off.
    // Installing fails harmlessly if a provider is already installed.
    let _ = rustls::crypto::ring::default_provider().install_default();

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

    let mut builder = Client::builder().tls_backend_rustls();
    for der in extra_roots {
        // Test builds only (see `Engine::with_test_root`).
        builder =
            builder.tls_certs_merge([reqwest::Certificate::from_der(der).map_err(Error::Http)?]);
    }
    builder
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

fn lock(file: &File, final_path: &Path) -> Result<(), Error> {
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

/// The first state for a whole-file response.
fn state_from_response(url: String, response: &Response) -> DownloadState {
    let headers = response.headers();
    let accept_ranges = header_str(headers, header::ACCEPT_RANGES)
        .is_some_and(|v| v.split(',').any(|u| u.trim().eq_ignore_ascii_case("bytes")));
    DownloadState::new(
        url,
        without_credentials(response.url()),
        validator_from(headers),
        response.content_length(),
        accept_ranges,
    )
}

/// The `If-Range` value RFC 9110 §13.1.5 allows, or `None` when there is no
/// strong validator and the download must start over rather than risk
/// mixing two versions of the file:
/// - a strong `ETag` (a weak one, `W/"…"`, must never be sent);
/// - only when there is **no** `ETag` at all, `Last-Modified`, and only if
///   it is strong (§8.8.2.2): at least one second before the `Date` of the
///   same response. A file changed twice within that second would otherwise
///   look unchanged.
fn if_range_value(validator: &Validator) -> Option<String> {
    match &validator.etag {
        Some(etag) if !etag.trim_start().starts_with("W/") => Some(etag.clone()),
        Some(_weak) => None,
        None => {
            let last_modified = validator.last_modified.as_ref()?;
            let modified = http_date::parse(last_modified)?;
            let date = http_date::parse(validator.date.as_ref()?)?;
            let gap = date.checked_sub(modified)?;
            (gap >= 1).then(|| last_modified.clone())
        }
    }
}

/// A `206` continues our download only if its `Content-Range` starts where
/// our data ends, covers the same total size, and the validator matches.
fn continues_at(response: &Response, state: &DownloadState, done: u64) -> bool {
    let range = header_str(response.headers(), header::CONTENT_RANGE).and_then(parse_content_range);
    let Some(ContentRange { start, end, total }) = range else {
        return false;
    };
    let same_total = match (total, state.content_length) {
        (Some(total), Some(expected)) => total == expected && end + 1 == total,
        (None, None) => true,
        _ => false,
    };
    start == done && same_total && !changed(state, response)
}

/// Whether `response` describes another version of the file than `state`.
fn changed(state: &DownloadState, response: &Response) -> bool {
    changed_from(&state.validator, state.content_length, response)
}

/// Whether `response` describes another version of the file than the one
/// with `validator` and `content_length`.
pub(crate) fn changed_from(
    validator: &Validator,
    content_length: Option<u64>,
    response: &Response,
) -> bool {
    let now = validator_from(response.headers());
    let differs = |old: &Option<String>, new: &Option<String>| matches!((old, new), (Some(old), Some(new)) if old != new);
    differs(&validator.etag, &now.etag)
        || differs(&validator.last_modified, &now.last_modified)
        || (response.status() == StatusCode::OK
            && response.content_length().is_some()
            && response.content_length() != content_length)
}

/// A parsed `Content-Range: bytes start-end/total` (RFC 9110 §14.4).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ContentRange {
    pub start: u64,
    pub end: u64,
    pub total: Option<u64>,
}

pub(crate) fn parse_content_range(value: &str) -> Option<ContentRange> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let (start, end): (u64, u64) = (start.trim().parse().ok()?, end.trim().parse().ok()?);
    let total = match total.trim() {
        "*" => None,
        n => Some(n.parse().ok()?),
    };
    (start <= end && total.is_none_or(|t| end < t)).then_some(ContentRange { start, end, total })
}

/// Saves the state file on a blocking thread: on Windows a save may wait up
/// to about a second for another program to release the file, and that wait
/// must not stop the async runtime. `new` is set for a new download that has
/// no data yet (see [`EmptyDownloadCleanup`]).
pub(crate) async fn save_state(
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

/// Runs a quick system query (such as the free space) on a thread of its
/// own: never on an async thread, and never queued behind slow file work on
/// the blocking pool, so a new download does not wait there before it has
/// created any file.
async fn on_own_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> io::Result<T> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("zajil-query".into())
        .spawn(move || {
            let _ = sender.send(work());
        })?;
    receiver
        .await
        .map_err(|_| io::Error::other("the query thread stopped"))
}

/// Runs blocking file-system work off the async threads.
pub(crate) async fn blocking<T: Send + 'static, E: From<io::Error> + Send + 'static>(
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
pub(crate) fn error_chain(err: &dyn std::error::Error) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    const FIVE_GIB: u64 = 5 * 1024 * 1024 * 1024;

    #[test]
    fn parses_content_range() {
        assert_eq!(
            parse_content_range("bytes 100-999/1000"),
            Some(ContentRange {
                start: 100,
                end: 999,
                total: Some(1000)
            })
        );
        assert_eq!(
            parse_content_range("bytes 0-9/*"),
            Some(ContentRange {
                start: 0,
                end: 9,
                total: None
            })
        );
        for bad in [
            "bytes 9-0/10",
            "bytes 0-10/10",
            "bytes */10",
            "items 0-1/2",
            "bytes 0-x/2",
            "",
        ] {
            assert_eq!(parse_content_range(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn handles_offsets_beyond_4_gib() {
        // Files larger than 4 GiB need 64-bit offsets end to end.
        let start = 4_500_000_000;
        let header = format!("bytes {start}-{}/{FIVE_GIB}", FIVE_GIB - 1);
        assert_eq!(
            parse_content_range(&header),
            Some(ContentRange {
                start,
                end: FIVE_GIB - 1,
                total: Some(FIVE_GIB)
            })
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.iso.zajil-state");
        let mut state = DownloadState::new(
            "http://h/huge.iso".into(),
            "http://h/huge.iso".into(),
            Validator {
                etag: Some("\"e\"".into()),
                last_modified: None,
                date: None,
            },
            Some(FIVE_GIB),
            true,
        );
        state.segments[0].done = start;
        state.save(&path).unwrap();
        let loaded = DownloadState::load(&path).unwrap();
        assert_eq!(loaded.bytes_done(), start);
        assert_eq!(loaded.segments[0].end, Some(FIVE_GIB - 1));
    }

    fn validator(etag: Option<&str>, modified: Option<&str>, date: Option<&str>) -> Validator {
        Validator {
            etag: etag.map(str::to_owned),
            last_modified: modified.map(str::to_owned),
            date: date.map(str::to_owned),
        }
    }

    const MODIFIED: &str = "Wed, 21 Oct 2026 07:28:00 GMT";
    const ONE_SECOND_LATER: &str = "Wed, 21 Oct 2026 07:28:01 GMT";

    #[test]
    fn if_range_uses_a_strong_etag() {
        let v = validator(Some("\"a\""), Some(MODIFIED), Some(ONE_SECOND_LATER));
        assert_eq!(if_range_value(&v).as_deref(), Some("\"a\""));
    }

    #[test]
    fn if_range_never_uses_a_weak_etag_nor_falls_back_to_the_date() {
        // Review of #17: the date was sent next to a weak ETag.
        for weak in ["W/\"a\"", " W/\"a\""] {
            let v = validator(Some(weak), Some(MODIFIED), Some(ONE_SECOND_LATER));
            assert_eq!(if_range_value(&v), None, "{weak}");
        }
    }

    #[test]
    fn if_range_uses_the_date_only_when_it_is_strong() {
        let strong = validator(None, Some(MODIFIED), Some(ONE_SECOND_LATER));
        assert_eq!(if_range_value(&strong).as_deref(), Some(MODIFIED));
        // Same second as the response: weak (RFC 9110 §8.8.2.2).
        let same_second = validator(None, Some(MODIFIED), Some(MODIFIED));
        assert_eq!(if_range_value(&same_second), None);
        // Unknown or unreadable Date: cannot prove it is strong.
        assert_eq!(if_range_value(&validator(None, Some(MODIFIED), None)), None);
        let garbage = validator(None, Some(MODIFIED), Some("soon"));
        assert_eq!(if_range_value(&garbage), None);
        let unreadable = validator(None, Some("last week"), Some(ONE_SECOND_LATER));
        assert_eq!(if_range_value(&unreadable), None);
        // Date before Last-Modified (clock trouble): weak.
        let backwards = validator(None, Some(ONE_SECOND_LATER), Some(MODIFIED));
        assert_eq!(if_range_value(&backwards), None);
        // Absurd dates are ignored, never compared (no overflow).
        let far = "Sun, 06 Nov 9223372036854775807 08:49:37 GMT";
        assert_eq!(
            if_range_value(&validator(None, Some(MODIFIED), Some(far))),
            None
        );
        assert_eq!(
            if_range_value(&validator(None, Some(far), Some(MODIFIED))),
            None
        );
        assert_eq!(if_range_value(&Validator::default()), None);
    }

    #[test]
    fn places_the_finished_file_next_to_a_taken_name() {
        let dir = tempfile::tempdir().unwrap();
        let wanted = dir.path().join("a.zip");
        std::fs::write(&wanted, b"old").unwrap();
        std::fs::write(dir.path().join("a (1).zip"), b"old").unwrap();
        // `a (2).zip` is another download in progress.
        std::fs::write(dir.path().join("a (2).zip.zajil-part"), b"").unwrap();
        let part = part_path(&wanted);
        std::fs::write(&part, b"new").unwrap();
        let placed = place_final(&part, &wanted).unwrap();
        assert_eq!(placed, dir.path().join("a (3).zip"));
        assert_eq!(std::fs::read(&wanted).unwrap(), b"old");
        assert_eq!(std::fs::read(placed).unwrap(), b"new");
    }
}
