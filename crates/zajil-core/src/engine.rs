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
//! 5. A request or a connection that fails for a reason that may pass (a
//!    broken connection, a timeout, `5xx`, `408`, `429`) is tried again
//!    after a growing wait (`retry.rs`, the policy of M2). If the transfer
//!    still stops early, the state is saved and the error says how to
//!    resume. Otherwise the part file is renamed to its final name in one
//!    step and the state file is removed.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use reqwest::header::{self, HeaderMap};
use reqwest::{Client, Response, StatusCode, Url, redirect};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::http_date;
use crate::limit::{Clock, Limits, SpeedLimit, SystemClock};
use crate::naming::{ReserveError, reserve_target, suggested_name, with_type_extension};
use crate::sanitize::{ConfineError, MAX_NAME_BYTES, confine, sanitize_file_name};
use crate::segmented::{self, Job, Outcome};
use crate::state::{
    DownloadState, StateError, Validator, final_path_for_state, part_path, state_path,
};

/// How many times one call may start over because the file changed on the
/// server, before giving up for now.
const MAX_RESTARTS: u8 = 3;

/// What one call of the engine may still do as it starts over: how many
/// restarts are left, whether the file may still be split, and how many
/// requests or connections failed in a row. Not `Debug`: it holds the user
/// name and password of the URL.
#[derive(Clone)]
struct Attempt {
    restarts_left: u8,
    split: bool,
    /// The URL the user gave, if it has a user name or password. Kept in
    /// memory only, never in the state file (ADR-0004, rule 7), so range
    /// requests carry them too.
    credentials: Option<Url>,
    /// Failures in a row (requests, and connections of a single-connection
    /// download) since the download last reached further into the file.
    failures: u32,
    /// The furthest byte a single connection has written in this call. A
    /// connection that breaks before passing it made no progress: a server
    /// that cannot continue from the middle and always breaks at the same
    /// place is given up, not tried forever.
    furthest: u64,
    /// Set when the caller wants the download to stop ([`Stop`]).
    stop: Stop,
    /// The speed limits every byte of this download goes through.
    limits: Limits,
    /// The counter of the download's site, if the user limited it
    /// ([`crate::sites`]): further connections of a split download take a
    /// slot of it each.
    site: Option<Arc<tokio::sync::Semaphore>>,
}

impl Attempt {
    fn new(url: Option<&Url>, stop: &Stop, limits: &Limits) -> Self {
        Attempt {
            stop: stop.clone(),
            limits: limits.clone(),
            restarts_left: MAX_RESTARTS,
            split: true,
            credentials: url
                .filter(|url| !url.username().is_empty() || url.password().is_some())
                .cloned(),
            failures: 0,
            furthest: 0,
            site: None,
        }
    }

    fn with_site(mut self, site: Option<Arc<tokio::sync::Semaphore>>) -> Self {
        self.site = site;
        self
    }

    /// Counts a failure of a single connection that had written up to
    /// `done`. `false` when the download must give up: `max_failures`
    /// failures in a row without reaching further into the file.
    fn connection_failed(&mut self, done: u64, max_failures: u32) -> bool {
        if done > self.furthest {
            self.furthest = done;
            self.failures = 0;
        }
        self.failed(max_failures)
    }

    /// Counts a failure; `false` once there were `max_failures` in a row.
    fn failed(&mut self, max_failures: u32) -> bool {
        self.failures = self.failures.saturating_add(1);
        self.failures < max_failures
    }

    /// For a start from byte 0 after a broken connection on a server that
    /// cannot continue from the middle: that is not a new version of the
    /// file, so it does not use up one of the restarts that [`Engine::start`]
    /// counts.
    fn after_broken_connection(self) -> Self {
        Attempt {
            restarts_left: self.restarts_left.saturating_add(1),
            ..self
        }
    }

    /// `url` with the user's credentials, if it is on the same origin as the
    /// URL that had them (as `reqwest` keeps them only within one origin).
    fn authorize(&self, url: &Url) -> Url {
        let mut url = url.clone();
        if let Some(with) = &self.credentials
            && with.origin() == url.origin()
        {
            let _ = url.set_username(with.username());
            let _ = url.set_password(with.password());
        }
        url
    }
}

/// Asks a running download to stop and lets its caller know when it has
/// ([`Engine::download_until`], [`Engine::resume_until`]). Cheap to clone:
/// every clone is the same signal.
#[derive(Debug, Clone)]
pub struct Stop {
    sender: Arc<tokio::sync::watch::Sender<bool>>,
}

impl Default for Stop {
    fn default() -> Self {
        Stop::new()
    }
}

impl Stop {
    /// A signal that nothing has requested yet.
    pub fn new() -> Self {
        Stop {
            sender: Arc::new(tokio::sync::watch::Sender::new(false)),
        }
    }

    /// Asks every download that uses this signal to stop.
    pub fn request(&self) {
        self.sender.send_replace(true);
    }

    pub fn is_requested(&self) -> bool {
        *self.sender.borrow()
    }

    /// Resolves once a stop is requested (at once if it already was).
    pub async fn requested(&self) {
        let mut receiver = self.sender.subscribe();
        // The sender lives in `self`, so this never fails.
        let _ = receiver.wait_for(|stop| *stop).await;
    }
}

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
    /// No segment is made smaller than this many bytes, at the start or by
    /// dynamic splitting.
    pub min_segment_size: u64,
    /// A request or a connection that fails this many times in a row,
    /// without the download reaching further in between, is given up: the
    /// download stops and can be resumed later. At least 1 (1 means no
    /// retry). The waits in between grow (`retry::backoff`).
    pub max_failures: u32,
    /// A limit shared by every download of this engine and its clones:
    /// the program-wide speed limit. Its rate can change while they run.
    pub speed_limit: Option<SpeedLimit>,
    /// Bytes per second for each download on its own.
    pub download_limit: Option<u64>,
    /// Where the limits take their time from (tests use a
    /// [`crate::limit::ManualClock`]).
    pub clock: Arc<dyn Clock>,
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
            max_failures: crate::retry::MAX_FAILURES,
            speed_limit: None,
            download_limit: None,
            clock: Arc::new(SystemClock::default()),
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
    /// Dynamic splitting: a free connection took over the second half of
    /// what remained of the slowest segment. The segment that starts at
    /// `segment` now ends just before `at`, where a new one starts.
    Split { segment: u64, at: u64 },
    /// The server answered `status` (`403 Forbidden` or `429 Too Many
    /// Requests`): at most `connections` connections from now on, and no
    /// new request for `wait`.
    ServerBusy {
        status: u16,
        connections: usize,
        wait: Option<Duration>,
    },
    /// A request or a single connection failed for `reason`, which may
    /// pass: it is tried again after `wait`.
    Retrying { wait: Duration, reason: String },
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

/// A page [`Engine::fetch_page`] fetched (M7, the site grabber).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedPage {
    /// The status the server answered, redirects included.
    pub status: u16,
    /// A redirect's `Location` (301, 302, 303, 307 or 308), as the server
    /// sent it: never followed, so the caller can check the target before
    /// asking for it. `None` for a redirect whose `Location` is missing or
    /// not text: a dead end.
    pub location: Option<String>,
    /// The media type, lowercase, without parameters (`text/html`).
    pub content_type: Option<String>,
    /// The name the file would get (as [`Probe::name`]).
    pub name: String,
    /// The body, read only for a successful HTML page (`text/html` or
    /// `application/xhtml+xml`; any successful answer for
    /// [`Engine::fetch_robots_until`]), and at most the size asked for.
    pub body: Option<Vec<u8>>,
    /// The body was longer than the size asked for: the rest was not read.
    pub truncated: bool,
}

/// What [`Engine::probe`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    /// The file's name, as a download would give it (sanitized).
    pub name: String,
    /// Its size, if the server said.
    pub size: Option<u64>,
    /// It can be split over several connections and resumed later.
    pub resumable: bool,
    /// The server announced `Accept-Ranges: bytes` (resumable also needs a
    /// size and a strong validator; the add dialog says which is missing).
    pub ranges: bool,
    /// The host of the final URL (after redirects), for categories.
    pub host: Option<String>,
    /// Its media type (`Content-Type` without parameters, lower case), if
    /// the server said: a sign-in page a link redirected to is `text/html`.
    pub content_type: Option<String>,
}

impl Probe {
    /// The server answered with a web page, not a file: a sign-in page a
    /// link redirected to, or a file's page on a site. Interfaces warn
    /// before downloading it.
    pub fn is_page(&self) -> bool {
        self.content_type.as_deref().is_some_and(is_page_type)
    }
}

/// Whether `media_type` is a web page: `text/html` or
/// `application/xhtml+xml`, in any case and with any parameters (a raw
/// `Content-Type` header, or [`Probe::content_type`]).
pub fn is_page_type(media_type: &str) -> bool {
    let media_type = media_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    matches!(media_type.as_str(), "text/html" | "application/xhtml+xml")
}

/// Why a new link is not shown to be the same file as a download's
/// ([`Error::NotTheSameFile`], M5 "refresh the download link"), for the
/// user in their language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mismatch {
    /// The download's server gave no strong validator (an `ETag`, or a
    /// strong `Last-Modified`), so no other link can be shown the same.
    NoValidator,
    /// The sizes differ, or one of them is unknown.
    Size {
        theirs: Option<u64>,
        ours: Option<u64>,
    },
    /// An `ETag` on one side, a date on the other.
    ValidatorKind,
    /// The new link gives no strong validator.
    NewNoValidator,
    /// Another version of the file: the validators differ.
    Version,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let size = |n: &Option<u64>| n.map_or("unknown".to_owned(), |n| format!("{n} bytes"));
        match self {
            Mismatch::NoValidator => f.write_str(
                "the server gave this download no strong validator (ETag or Last-Modified), \
                 so Zajil cannot tell that another link is the same file",
            ),
            Mismatch::Size { theirs, ours } => write!(
                f,
                "its size is {}, the download's {}",
                size(theirs),
                size(ours)
            ),
            Mismatch::ValidatorKind => f.write_str(
                "the server describes it differently (an ETag on one side, a date on the \
                 other), so Zajil cannot tell it is the same file",
            ),
            Mismatch::NewNoValidator => f.write_str(
                "the new link has no strong validator, so Zajil cannot tell it is the same file",
            ),
            Mismatch::Version => f.write_str(
                "the server says it is another version of the file (its validator differs)",
            ),
        }
    }
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
    #[error(
        "the server asks to wait {} s before more requests; try again later",
        .wait.as_secs()
    )]
    ServerBusy { wait: Duration },
    #[error("the download stopped ({reason}); resume it from {}", state_file.display())]
    Interrupted { state_file: PathBuf, reason: String },
    /// The caller asked the download to stop ([`Stop`]). `state_file` is
    /// where to resume it; `None` when nothing was written (a new download
    /// then leaves no files).
    #[error("{}", match state_file {
        Some(path) => format!("the download was stopped; resume it from {}", path.display()),
        None => "the download was stopped before any data was written".to_owned(),
    })]
    Stopped { state_file: Option<PathBuf> },
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
    /// Only for a download that was started with a user name or password
    /// in its URL (`had_credentials` in the state file); any other `401` is
    /// [`Error::Status`].
    #[error(
        "the server asks for a user name and password (401 Unauthorized); Zajil never saves \
         them in the state file, so this download cannot be resumed and has to be downloaded \
         again"
    )]
    CredentialsNeeded {
        /// The URL of the download, without a user name or password.
        url: String,
        state_file: PathBuf,
    },
    #[error("another process is already downloading {}", .0.display())]
    Locked(PathBuf),
    /// A download held for the user before anything was made: the name it
    /// would be saved under (the server's, or the user's) is one that
    /// [`Engine::holding`] holds.
    #[error("{0} can run code: it waits for you to start it")]
    Held(String),
    /// Google Drive answered with a page, not the file, and the page gave
    /// no link to it: the file is not shared with everyone, is over its
    /// download limit, or is not a file. Nothing was made.
    #[error(
        "Google Drive answered with a page, not the file: it may not be shared with everyone, \
         or it was downloaded too many times today; open it in the browser"
    )]
    DrivePage,
    /// "Refresh the link": the new link is not the same file (M5).
    #[error("the new link is not the same file: {0}")]
    NotTheSameFile(Mismatch),
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
    /// The same client, but it never follows a redirect: for
    /// [`Engine::fetch_page`], whose caller checks each target first.
    page_client: Client,
    options: Options,
    /// The browser's headers for downloads from this engine (M5).
    browser: Arc<crate::browser::FromBrowser>,
    /// The user's per-site connection limits, shared by every download of
    /// this engine and its clones ([`crate::sites`]).
    sites: Arc<crate::sites::SiteLimits>,
    /// Its site's slot, taken ahead by the queue runner (#170).
    taken_slot: crate::sites::TakenSlot,
    /// Hold a new download whose name it says: the name the server gives
    /// at the download itself, before any file is made (security review
    /// #84). `None`: no check.
    hold: Option<Hold>,
    /// The download's own limit, given by its caller so that its rate can
    /// change while it runs (#106); `None`: a fresh one from
    /// [`Options::download_limit`].
    own_limit: Option<SpeedLimit>,
    /// The name a new download is saved under instead of the server's
    /// (the user's, M5.5, #103). `None`: the server's.
    name: Option<String>,
    /// Where Google Drive's links and downloads are.
    drive: Arc<crate::drive::DriveSite>,
}

/// What [`Engine::holding`] checks a new download's names with.
#[derive(Clone)]
struct Hold(HoldCheck);

/// Whether to hold a new download, from the name the server gives and the
/// name it is saved under ([`Engine::named`]; the same when it has none).
pub type HoldCheck = Arc<dyn Fn(&str, &str) -> bool + Send + Sync>;

impl std::fmt::Debug for Hold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Hold")
    }
}

impl Engine {
    pub fn new(options: Options) -> Result<Self, Error> {
        let client = build_client(&options, &[], Redirects::Follow)?;
        let page_client = build_client(&options, &[], Redirects::Never)?;
        Ok(Engine {
            client,
            page_client,
            options,
            browser: Arc::default(),
            sites: Arc::default(),
            taken_slot: Default::default(),
            hold: None,
            own_limit: None,
            name: None,
            drive: Arc::default(),
        })
    }

    /// **Test builds only.** Like [`Engine::new`], but also trusts the
    /// certificate authority `ca_der` (DER), next to the system store.
    ///
    /// Compiled only with the `test-support` feature, which only
    /// `dev-dependencies` enable: the shipped `zajil` has no way to add a
    /// trusted root, and CI checks that its release build lacks the feature.
    #[cfg(feature = "test-support")]
    pub fn with_test_root(options: Options, ca_der: &[u8]) -> Result<Self, Error> {
        let client = build_client(&options, &[ca_der.to_vec()], Redirects::Follow)?;
        let page_client = build_client(&options, &[ca_der.to_vec()], Redirects::Never)?;
        Ok(Engine {
            client,
            page_client,
            options,
            browser: Arc::default(),
            sites: Arc::default(),
            taken_slot: Default::default(),
            hold: None,
            own_limit: None,
            name: None,
            drive: Arc::default(),
        })
    }

    /// **Test builds only.** This engine, taking `links` for Google
    /// Drive's files' pages and `download` for its download address (a
    /// local test server's).
    #[cfg(feature = "test-support")]
    pub fn with_test_drive(&self, links: &str, download: &str) -> Engine {
        let mut engine = self.clone();
        engine.drive = Arc::new(crate::drive::DriveSite::new(
            Url::parse(links).expect("a valid URL"),
            Url::parse(download).expect("a valid URL"),
        ));
        engine
    }

    /// This engine, with `limit` shared by all its downloads (and those of
    /// its clones) as the program-wide speed limit.
    pub fn with_speed_limit(&self, limit: Option<SpeedLimit>) -> Engine {
        let mut engine = self.clone();
        engine.options.speed_limit = limit;
        engine
    }

    /// This engine, sending `from` (the browser's cookies for one origin
    /// and the page the link was on) with every request of its downloads.
    /// Kept in memory only: the state file never holds them (owner's
    /// decision on #7).
    pub fn with_browser(&self, from: crate::browser::FromBrowser) -> Engine {
        let mut engine = self.clone();
        engine.browser = Arc::new(from);
        engine
    }

    /// This engine, counting the connections to each site in `sites` (the
    /// settings file's `site_connections`) across all its downloads and
    /// those of its clones made after this.
    pub fn with_site_limits(&self, sites: crate::sites::SiteLimits) -> Engine {
        let mut engine = self.clone();
        engine.sites = Arc::new(sites);
        engine
    }

    /// The counter of `url`'s site, if the user limited it.
    pub(crate) fn site_counter(&self, url: &str) -> Option<Arc<tokio::sync::Semaphore>> {
        self.sites.counter_for(&Url::parse(url).ok()?)
    }

    /// This engine, for one download whose site's slot the queue runner
    /// took ahead.
    pub(crate) fn with_taken_slot(&self, permit: tokio::sync::OwnedSemaphorePermit) -> Engine {
        let mut engine = self.clone();
        engine.taken_slot = crate::sites::TakenSlot::new(permit);
        engine
    }

    /// This engine, holding a new download whose names make `hold` true
    /// (the server's, then the one it is saved under): it stops with
    /// [`Error::Held`] once the server has named the file, before anything
    /// is created. For a file from a page that can run code: the probe's
    /// name is not enough, the download's own answer may name another file
    /// (review of #90).
    pub fn holding(&self, hold: HoldCheck) -> Engine {
        let mut engine = self.clone();
        engine.hold = Some(Hold(hold));
        engine
    }

    /// This engine, saving a new download under `name` instead of the name
    /// the server gives (`None`: the server's). Cleaned like any name
    /// ([`sanitize_file_name`]), and numbered like any name if it is
    /// taken. A resumed download keeps the name it has.
    pub fn named(&self, name: Option<&str>) -> Engine {
        let mut engine = self.clone();
        engine.name = name.map(sanitize_file_name);
        engine
    }

    /// This engine, with `connections` for each download it starts
    /// (clamped like [`Options::connections`]): for a setting the user
    /// changes while the program runs (M4).
    pub fn with_connections(&self, connections: usize) -> Engine {
        let mut engine = self.clone();
        engine.options.connections = connections.clamp(1, crate::MAX_CONNECTIONS);
        engine
    }

    /// The clock the engine's limits use.
    pub fn clock(&self) -> Arc<dyn Clock> {
        self.options.clock.clone()
    }

    /// This engine, with `rate` bytes per second for each download (`None`:
    /// no limit of its own), instead of a limit given with
    /// [`Engine::with_own_limit`]. The program-wide limit stays shared.
    pub fn with_download_limit(&self, rate: Option<u64>) -> Engine {
        let mut engine = self.clone();
        engine.options.download_limit = rate;
        engine.own_limit = None;
        engine
    }

    /// This engine, with `limit` as the limit of its own of the download it
    /// starts (instead of [`Options::download_limit`]): the caller keeps a
    /// clone and changes its rate while the download runs (#106). Every
    /// download this engine and its clones start shares it, so it is for
    /// one download. The program-wide limit stays shared. It wins over
    /// [`Engine::with_download_limit`], which clears it.
    pub fn with_own_limit(&self, limit: SpeedLimit) -> Engine {
        let mut engine = self.clone();
        engine.own_limit = Some(limit);
        engine
    }

    /// The limits a new download goes through: the program's, shared, and
    /// its own: the one given ([`Engine::with_own_limit`]), or a fresh one.
    fn limits(&self) -> Limits {
        let own = self.own_limit.clone().or_else(|| {
            self.options
                .download_limit
                .map(|rate| SpeedLimit::new(Some(rate), self.options.clock.clone()))
        });
        Limits {
            limits: self
                .options
                .speed_limit
                .iter()
                .cloned()
                .chain(own)
                .collect(),
        }
    }

    /// Downloads `url` into the folder `dir` (created if it does not exist,
    /// since M4: queue items whose folder was removed, and folders kept from
    /// an imported file).
    pub async fn download(
        &self,
        url: &str,
        dir: &Path,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        self.download_until(url, dir, &Stop::new(), on_event).await
    }

    /// Like [`Engine::download`], until `stop` is requested: then what is on
    /// disk is synced and recorded (unless a periodic save already did:
    /// the state file counts only synced bytes, #132), and the call returns
    /// [`Error::Stopped`] only once nothing of the download runs any more
    /// (no write, no save, the part file closed and unlocked). So the
    /// download can be resumed at once in the same process, and no older
    /// save can land after the resume's own (#5).
    ///
    /// Dropping the future instead also stops it, but its last writes and
    /// saves may still finish on blocking threads afterwards.
    pub async fn download_until(
        &self,
        url: &str,
        dir: &Path,
        stop: &Stop,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        self.download_choosing_until(url, &|_, _| dir.to_owned(), stop, on_event)
            .await
    }

    /// Like [`Engine::download_until`], but the folder is chosen by
    /// `choose` once the file's name and the final URL (after redirects)
    /// are known, before any file is created: categories (M4) need both.
    /// The chosen folder is created if it does not exist.
    pub async fn download_choosing_until(
        &self,
        url: &str,
        choose: &(dyn Fn(&str, &reqwest::Url) -> PathBuf + Send + Sync),
        stop: &Stop,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        let url = parse_url(url)?;
        // The download's own slot of its site, held until it ends.
        let site = self.sites.counter_for(&url);
        let _slot = crate::sites::take_slot(site.as_ref(), &self.taken_slot, stop).await?;
        let mut attempt = Attempt::new(Some(&url), stop, &self.limits()).with_site(site);
        let response = self
            .retrying(&mut attempt, on_event, || self.get(url.clone(), None))
            .await?;
        if response.status() != StatusCode::OK {
            return Err(Error::Status(response.status()));
        }
        let server_name = response_name(&response);
        let name = self.name.clone().unwrap_or_else(|| server_name.clone());
        if self
            .hold
            .as_ref()
            .is_some_and(|hold| (hold.0)(&server_name, &name))
        {
            return Err(Error::Held(name));
        }
        let dir = choose(&name, response.url());
        // Only a folder that does not exist yet (a category's) is created:
        // creating one waits for a blocking thread, and a download into an
        // existing folder must reserve its name as soon as before (the
        // cancellation test with a busy blocking pool in `download.rs`
        // found this). The check itself is one quick `stat`.
        if !dir.is_dir() {
            tokio::fs::create_dir_all(&dir).await?;
        }
        let dir = dir.as_path();
        // The size comes from the server: before creating anything, make
        // sure it fits (a huge or false size fails here, clearly).
        if let Some(len) = response.content_length() {
            check_space(dir, len, 0).await?;
        }
        let (final_path, part) = reserve_target(dir, &name)?;
        // From here until the first byte is written, any failure or
        // cancellation removes the new files again.
        let cleanup = EmptyDownloadCleanup::new(&final_path);
        lock(&part, &final_path)?;
        let state = state_from_response(
            without_credentials(&url),
            attempt.credentials.is_some(),
            &response,
        );
        self.start(
            final_path,
            part,
            state,
            response,
            Some(cleanup),
            attempt,
            on_event,
        )
        .await
    }

    /// A small text resource at `url` (an HLS playlist), at most `limit`
    /// bytes, with the same TLS, redirects and browser headers as a
    /// download, and not saved anywhere. For the video module's own DRM
    /// check (ADR-0009, section 6). A larger answer is an error, never
    /// read past the limit.
    pub async fn fetch_text(&self, url: &str, limit: usize, stop: &Stop) -> Result<String, Error> {
        let url = parse_url(url)?;
        let fetch = async {
            let mut response = self.get(url, None).await?;
            if response.status() != StatusCode::OK {
                return Err(Error::Status(response.status()));
            }
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|err| Error::Http(without_secrets(err)))?
            {
                if body.len().saturating_add(chunk.len()) > limit {
                    return Err(Error::Io(io::Error::other(format!(
                        "the answer is larger than {limit} bytes"
                    ))));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(String::from_utf8_lossy(&body).into_owned())
        };
        tokio::select! {
            result = fetch => result,
            () = stop.requested() => Err(Error::Stopped { state_file: None }),
        }
    }

    /// What `url` holds, without downloading it: the name the file would
    /// get, its size, and whether it can be split and resumed (the server
    /// announces `Accept-Ranges: bytes`, a size and a strong validator).
    /// For the add dialog of the interface (M4). The first response is
    /// read for its headers only; the connection is then dropped.
    pub async fn probe(&self, url: &str) -> Result<Probe, Error> {
        self.probe_until(url, &Stop::new()).await
    }

    /// [`probe`](Self::probe), given up when `stop` is requested (the add
    /// dialog is closed while the server is tried again: review of #48).
    pub async fn probe_until(&self, url: &str, stop: &Stop) -> Result<Probe, Error> {
        let url = parse_url(url)?;
        let mut attempt = Attempt::new(Some(&url), stop, &self.limits());
        let response = self
            .retrying(&mut attempt, &mut |_| {}, || self.get(url.clone(), None))
            .await?;
        if response.status() != StatusCode::OK {
            return Err(Error::Status(response.status()));
        }
        let name = response_name(&response);
        let state = state_from_response(String::new(), false, &response);
        let size = response.content_length();
        let resumable = state.resumable();
        Ok(Probe {
            name,
            size,
            resumable,
            ranges: state.accept_ranges,
            host: response.url().host_str().map(str::to_owned),
            content_type: response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .map(|v| v.trim().to_ascii_lowercase())
                .filter(|v| !v.is_empty()),
        })
    }

    /// Fetches the page at `url` into memory, for the site grabber (M7,
    /// ADR-0010): **without following a redirect** (its `Location` is
    /// returned, for the caller to check), with the body read only for a
    /// successful HTML page, and at most `max_bytes` of it. One try; the
    /// same TLS checks, user agent and browser headers as a download.
    pub async fn fetch_page(&self, url: &str, max_bytes: usize) -> Result<FetchedPage, Error> {
        self.fetch_page_until(url, max_bytes, &Stop::new()).await
    }

    /// [`fetch_page`](Self::fetch_page), given up when `stop` is requested.
    pub async fn fetch_page_until(
        &self,
        url: &str,
        max_bytes: usize,
        stop: &Stop,
    ) -> Result<FetchedPage, Error> {
        self.fetch_page_inner(url, max_bytes, stop, ReadBody::Html)
            .await
    }

    /// Like [`fetch_page_until`](Self::fetch_page_until), but the body of
    /// any successful answer is read, whatever its type: for `robots.txt`,
    /// which servers send as `text/plain` or with no type at all. (Not
    /// [`fetch_text`](Self::fetch_text), which follows redirects and wants
    /// a 200.)
    pub async fn fetch_robots_until(
        &self,
        url: &str,
        max_bytes: usize,
        stop: &Stop,
    ) -> Result<FetchedPage, Error> {
        self.fetch_page_inner(url, max_bytes, stop, ReadBody::Any)
            .await
    }

    async fn fetch_page_inner(
        &self,
        url: &str,
        max_bytes: usize,
        stop: &Stop,
        read: ReadBody,
    ) -> Result<FetchedPage, Error> {
        let url = parse_url(url)?;
        let request = self.browser.apply(&url, self.page_client.get(url.clone()));
        let mut response = tokio::select! {
            result = request.send() => result.map_err(|err| Error::Http(without_secrets(err)))?,
            () = stop.requested() => return Err(Error::Stopped { state_file: None }),
        };
        let header_text = |name| {
            response
                .headers()
                .get(name)
                .and_then(|v: &header::HeaderValue| v.to_str().ok())
                .map(str::to_owned)
        };
        // Only a redirect's: a 300 or 304 that carries one leads nowhere.
        let location = header_text(header::LOCATION)
            .filter(|_| matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308));
        let content_type = header_text(header::CONTENT_TYPE)
            .and_then(|v| v.split(';').next().map(|v| v.trim().to_ascii_lowercase()))
            .filter(|v| !v.is_empty());
        let name = response_name(&response);
        let html = matches!(
            content_type.as_deref(),
            Some("text/html" | "application/xhtml+xml")
        );
        let mut page = FetchedPage {
            status: response.status().as_u16(),
            location,
            content_type,
            name,
            body: None,
            truncated: false,
        };
        if !(response.status().is_success() && (html || read == ReadBody::Any)) {
            return Ok(page);
        }
        let mut body = Vec::new();
        loop {
            let chunk = tokio::select! {
                chunk = response.chunk() => chunk.map_err(|err| Error::Http(without_secrets(err)))?,
                () = stop.requested() => return Err(Error::Stopped { state_file: None }),
            };
            let Some(chunk) = chunk else { break };
            let room = max_bytes.saturating_sub(body.len());
            if chunk.len() > room {
                body.extend_from_slice(&chunk[..room]);
                page.truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }
        page.body = Some(body);
        Ok(page)
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
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        self.resume_until(state_file, &Stop::new(), on_event).await
    }

    /// Like [`Engine::resume`], until `stop` is requested (see
    /// [`Engine::download_until`]).
    pub async fn resume_until(
        &self,
        state_file: &Path,
        stop: &Stop,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        let limits = self.limits();
        let final_path = checked_final_path(state_file)?;
        let state_file = state_path(&final_path);
        let state = DownloadState::load(&state_file)?;
        // The download's own slot of its site, held until it ends. Stopped
        // while waiting: still resumable from its state file.
        let site = Url::parse(&state.url)
            .ok()
            .and_then(|url| self.sites.counter_for(&url));
        let _slot = crate::sites::take_slot(site.as_ref(), &self.taken_slot, stop)
            .await
            .map_err(|_| Error::Stopped {
                state_file: Some(state_file.clone()),
            })?;
        // Read again once the slot came, as the wait may be long and the
        // part file is not locked meanwhile: if another resume finished it,
        // the state file is gone and nothing is downloaded again (review of
        // #166).
        let state = if site.is_some() {
            DownloadState::load(&state_file)?
        } else {
            state
        };
        let new_attempt = || Attempt::new(None, stop, &limits).with_site(site.clone());
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
                    new_attempt(),
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
                    new_attempt(),
                    on_event,
                )
                .await;
        }
        let if_range = if_range.expect("checked above");
        if state.segments.len() > 1 {
            return self
                .resume_segments(final_path, part, state, if_range, new_attempt(), on_event)
                .await;
        }

        let mut attempt = new_attempt();
        let response = self
            .get_with_fallback(&state, Some((done, None, if_range)), &mut attempt, on_event)
            .await
            .map_err(|err| resumable_stop(err, &final_path, false))?;
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
                            attempt,
                            on_event,
                        )
                        .await;
                }
                let mut part = part;
                part.seek(SeekFrom::Start(done))?;
                let transfer = self.transfer(final_path, part, state, None);
                self.receive(transfer, response, attempt, on_event).await
            }
            StatusCode::OK => {
                let reason = if changed(&state, &response) {
                    RestartReason::FileChanged
                } else {
                    RestartReason::RangesIgnored
                };
                on_event(Event::Restarted { reason });
                let state =
                    state_from_response(state.url.clone(), state.had_credentials, &response);
                self.start(final_path, part, state, response, None, attempt, on_event)
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
                    attempt,
                    on_event,
                )
                .await
            }
            status => Err(refused(status, &state, &final_path, &attempt)),
        }
    }

    /// Gives the download of `state_file` a new link (`new_url`): the same
    /// file from another address, as when a signed link expired (M5,
    /// "refresh the download link"). The server must show it is the same
    /// file: the same size and the same strong validator (an `ETag`, or a
    /// strong `Last-Modified`) as the state file. Otherwise
    /// [`Error::NotTheSameFile`], and the state file is left as it was. Only
    /// the first response's headers are read; the download then goes on
    /// with [`Engine::resume_until`], asking only for what is missing.
    ///
    /// The download must not be running ([`Error::Locked`] otherwise), and
    /// `new_url` must be an http or https URL without a user name or
    /// password.
    pub async fn refresh_link(&self, state_file: &Path, new_url: &str) -> Result<(), Error> {
        self.refresh_link_until(state_file, new_url, &Stop::new())
            .await
    }

    /// [`Engine::refresh_link`], given up when `stop` is requested
    /// ([`Error::Stopped`], the state file left as it was): the user closed
    /// the dialog while the new link's server was slow (review of #78).
    pub async fn refresh_link_until(
        &self,
        state_file: &Path,
        new_url: &str,
        stop: &Stop,
    ) -> Result<(), Error> {
        let final_path = checked_final_path(state_file)?;
        let state_file = state_path(&final_path);
        let url = parse_url(new_url)?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(Error::InvalidUrl {
                url: shown_url(new_url),
                reason: "a new link with a user name or password is not accepted".into(),
            });
        }
        // The part file's lock is held from before the state is read until
        // after it is written: a running download holds it. With no part
        // file, one is made empty to hold it (the resume then starts over,
        // as it would have), and removed again if the refresh fails
        // (review of #74).
        let part_file = part_path(&final_path);
        let (part, made) = match zajil_platform::fs::open_regular_nofollow(&part_file, true) {
            Ok(part) => (part, false),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let part = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&part_file)?;
                (part, true)
            }
            Err(err) if zajil_platform::fs::is_not_regular_file(&err) => {
                return Err(Error::NotRegularFile(part_file));
            }
            Err(err) => return Err(err.into()),
        };
        lock(&part, &final_path)?;
        let result = self.refresh_locked(&state_file, &url, stop).await;
        if result.is_err() && made {
            drop(part);
            let _ = std::fs::remove_file(&part_file);
        }
        result
    }

    /// [`Engine::refresh_link`], with the part file locked.
    async fn refresh_locked(&self, state_file: &Path, url: &Url, stop: &Stop) -> Result<(), Error> {
        let mut state = DownloadState::load(state_file)?;
        let Some(ours) = if_range_value(&state.validator) else {
            return Err(Error::NotTheSameFile(Mismatch::NoValidator));
        };
        let limits = self.limits();
        let mut attempt = Attempt::new(None, stop, &limits);
        let response = self
            .retrying(&mut attempt, &mut |_| {}, || self.get(url.clone(), None))
            .await?;
        if response.status() != StatusCode::OK {
            return Err(Error::Status(response.status()));
        }
        // Built as a download's state is: the final URL without a user
        // name or password (review of #74: a redirect's were kept), and the
        // link as the URL parser writes it, as a download's and a queue
        // item's are (a queue resumes a state only for its item's link).
        let theirs = state_from_response(without_credentials(url), false, &response);
        drop(response);
        if state.content_length.is_none() || theirs.content_length != state.content_length {
            return Err(Error::NotTheSameFile(Mismatch::Size {
                theirs: theirs.content_length,
                ours: state.content_length,
            }));
        }
        match if_range_value(&theirs.validator) {
            Some(new) if new == ours => {}
            // Another kind of validator, or none: not shown to differ, but
            // nothing shows it is the same file either.
            Some(new) if new.starts_with('"') != ours.starts_with('"') => {
                return Err(Error::NotTheSameFile(Mismatch::ValidatorKind));
            }
            None => return Err(Error::NotTheSameFile(Mismatch::NewNoValidator)),
            Some(_) => return Err(Error::NotTheSameFile(Mismatch::Version)),
        }
        state.url = theirs.url;
        state.final_url = theirs.final_url;
        state.had_credentials = false;
        state.save(state_file)?;
        Ok(())
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
        attempt: Attempt,
        on_event: &mut (dyn FnMut(Event) + Send),
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
        let mut attempt = attempt;
        let response = self
            .get_with_fallback(
                &state,
                Some((from, Some(to), if_range.clone())),
                &mut attempt,
                on_event,
            )
            .await
            .map_err(|err| resumable_stop(err, &final_path, false))?;
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
                        .restart(final_path, part, state, reason, None, attempt, on_event)
                        .await;
                }
                let job = Job {
                    client: self.client.clone(),
                    browser: self.browser.clone(),
                    connections: self.options.connections,
                    min_segment: self.options.min_segment_size,
                    save_interval: self.options.save_interval,
                    max_failures: self.options.max_failures,
                    stop: attempt.stop.clone(),
                    limits: attempt.limits.clone(),
                    site: attempt.site.clone(),
                    url: response.url().clone(),
                    if_range,
                    final_path,
                    part,
                    state,
                    first: Some((response, segment.start)),
                    cleanup: None,
                };
                self.run_segments(job, attempt, on_event).await
            }
            StatusCode::OK => {
                let reason = if changed(&state, &response) {
                    RestartReason::FileChanged
                } else {
                    RestartReason::RangesIgnored
                };
                on_event(Event::Restarted { reason });
                let state =
                    state_from_response(state.url.clone(), state.had_credentials, &response);
                self.start(final_path, part, state, response, None, attempt, on_event)
                    .await
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                self.restart(
                    final_path,
                    part,
                    state,
                    RestartReason::RangeMismatch,
                    None,
                    attempt,
                    on_event,
                )
                .await
            }
            status => Err(refused(status, &state, &final_path, &attempt)),
        }
    }

    /// Runs a segmented download, and starts over if the file changed.
    async fn run_segments(
        &self,
        job: Job,
        attempt: Attempt,
        on_event: &mut (dyn FnMut(Event) + Send),
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
            } => {
                // The validator changed while segments were being asked for.
                // Some servers have several machines behind one name, each
                // with its own ETag for the same file: one connection reads
                // from one machine, so start over on one connection.
                let attempt = Attempt {
                    split: attempt.split && reason != RestartReason::FileChanged,
                    ..attempt
                };
                // A page Drive answered a connection with (its file was
                // downloaded too often, its link expired) is not the file:
                // start over through `get`, which follows or refuses it.
                let response =
                    response.filter(|r| !(is_bare_page(r) && self.is_drive_download(&old)));
                match response {
                    Some(response) => {
                        on_event(Event::Restarted { reason });
                        let state = state_from_response(old.url, old.had_credentials, &response);
                        Box::pin(self.start(
                            final_path, part, state, response, cleanup, attempt, on_event,
                        ))
                        .await
                    }
                    None => {
                        Box::pin(
                            self.restart(final_path, part, old, reason, cleanup, attempt, on_event),
                        )
                        .await
                    }
                }
            }
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
        attempt: Attempt,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        on_event(Event::Restarted { reason });
        let mut attempt = attempt;
        let response = self
            .get_with_fallback(&old, None, &mut attempt, on_event)
            .await
            .map_err(|err| resumable_stop(err, &final_path, cleanup.is_some()))?;
        if response.status() != StatusCode::OK {
            return Err(refused(response.status(), &old, &final_path, &attempt));
        }
        let state = state_from_response(old.url, old.had_credentials, &response);
        self.start(
            final_path, part, state, response, cleanup, attempt, on_event,
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
        attempt: Attempt,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        let Some(restarts_left) = attempt.restarts_left.checked_sub(1) else {
            return Err(Error::Interrupted {
                state_file: state_path(&final_path),
                reason: "the file kept changing on the server; try again later".into(),
            });
        };
        let attempt = Attempt {
            restarts_left,
            ..attempt
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

        let split = attempt.split.then(|| self.split(&state)).flatten();
        let watch = cleanup.as_ref().and_then(EmptyDownloadCleanup::watch);
        if let Some(segments) = split {
            state.segments = segments;
            save_state(&mut state, &state_path(&final_path), watch).await?;
            let if_range = if_range_value(&state.validator).expect("checked by split");
            let job = Job {
                client: self.client.clone(),
                browser: self.browser.clone(),
                connections: self.options.connections,
                min_segment: self.options.min_segment_size,
                save_interval: self.options.save_interval,
                max_failures: self.options.max_failures,
                stop: attempt.stop.clone(),
                limits: attempt.limits.clone(),
                site: attempt.site.clone(),
                url: attempt.authorize(response.url()),
                if_range,
                final_path,
                part,
                state,
                first: Some((response, 0)),
                cleanup,
            };
            return self.run_segments(job, attempt, on_event).await;
        }
        save_state(&mut state, &state_path(&final_path), watch).await?;
        let transfer = self.transfer(final_path, part, state, cleanup);
        self.receive(transfer, response, attempt, on_event).await
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
            saved: false,
            cleanup,
        }
    }

    /// Writes the body of `response` into `transfer` from its current
    /// position, then finishes.
    ///
    /// A connection that breaks is tried again (M3, owner's decision on
    /// #28), with the policy of the segmented connections (`retry.rs`): what
    /// is on disk is synced and recorded, and after a growing wait the rest
    /// is asked for with `Range` and `If-Range`. If the server cannot
    /// continue from there (no strong validator, ranges ignored, or the file
    /// changed), the download starts again from byte 0 with the whole-file
    /// answer: old and new data are never mixed. After `max_failures`
    /// failures in a row without reaching further into the file, the
    /// download stops and can be resumed later.
    async fn receive(
        &self,
        mut transfer: Transfer,
        mut response: Response,
        mut attempt: Attempt,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        on_event(Event::Started {
            file: transfer.final_path.clone(),
            state_file: transfer.state_file.clone(),
            total: transfer.state.content_length,
            from: transfer.done(),
        });
        loop {
            let stop = attempt.stop.clone();
            let reason = match transfer
                .pump(&mut response, &stop, &attempt.limits, on_event)
                .await
            {
                Ok(()) => return transfer.finish(on_event).await,
                Err(Halt::Fatal(reason)) => return Err(transfer.interrupt(reason).await),
                Err(Halt::Requested) => return Err(transfer.stopped().await),
                Err(Halt::Broken(reason)) => reason,
            };
            drop(response);
            let done = transfer.done();
            if !attempt.connection_failed(done, self.options.max_failures) {
                let failures = attempt.failures;
                return Err(transfer
                    .interrupt(format!("{reason} (gave up after {failures} tries)"))
                    .await);
            }
            // A new download with no data yet has nothing to record.
            if transfer.cleanup.is_none()
                && let Err(err) = transfer.checkpoint().await
            {
                return Err(transfer.interrupt(err.to_string()).await);
            }
            let wait = crate::retry::backoff(attempt.failures);
            on_event(Event::Retrying { wait, reason });
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = stop.requested() => return Err(transfer.stopped().await),
            }

            let if_range = if done > 0 {
                if_range_value(&transfer.state.validator)
            } else {
                None
            };
            let range = if_range.clone().map(|value| (done, None, value));
            response = match self
                .get_with_fallback(&transfer.state, range, &mut attempt, on_event)
                .await
            {
                Ok(response) => response,
                Err(Error::Stopped { .. }) => return Err(transfer.stopped().await),
                Err(err) => return Err(transfer.interrupt(err.to_string()).await),
            };
            match response.status() {
                StatusCode::PARTIAL_CONTENT
                    if if_range.is_some() && continues_at(&response, &transfer.state, done) =>
                {
                    if let Err(err) = transfer.file.seek(SeekFrom::Start(done)).await {
                        return Err(transfer.interrupt(err.to_string()).await);
                    }
                }
                StatusCode::OK => {
                    // Nothing was written yet: simply the first answer again.
                    let reason = if done == 0 {
                        None
                    } else if if_range.is_none() {
                        Some(RestartReason::NoValidator)
                    } else if changed(&transfer.state, &response) {
                        Some(RestartReason::FileChanged)
                    } else {
                        Some(RestartReason::RangesIgnored)
                    };
                    if let Some(reason) = reason {
                        on_event(Event::Restarted { reason });
                    }
                    let state = state_from_response(
                        transfer.state.url.clone(),
                        transfer.state.had_credentials,
                        &response,
                    );
                    let attempt = if reason == Some(RestartReason::FileChanged) {
                        attempt
                    } else {
                        attempt.after_broken_connection()
                    };
                    let (final_path, part, cleanup) = transfer.into_parts().await;
                    return Box::pin(self.start(
                        final_path, part, state, response, cleanup, attempt, on_event,
                    ))
                    .await;
                }
                StatusCode::RANGE_NOT_SATISFIABLE
                    if if_range.is_some() && transfer.state.content_length == Some(done) =>
                {
                    // Everything was already on disk.
                    return transfer.finish(on_event).await;
                }
                StatusCode::PARTIAL_CONTENT | StatusCode::RANGE_NOT_SATISFIABLE => {
                    let old = transfer.state.clone();
                    let (final_path, part, cleanup) = transfer.into_parts().await;
                    return Box::pin(self.restart(
                        final_path,
                        part,
                        old,
                        RestartReason::RangeMismatch,
                        cleanup,
                        attempt,
                        on_event,
                    ))
                    .await;
                }
                status => {
                    return Err(
                        match refused(status, &transfer.state, &transfer.final_path, &attempt) {
                            err @ Error::CredentialsNeeded { .. } => err,
                            err => transfer.interrupt(err.to_string()).await,
                        },
                    );
                }
            }
        }
    }

    /// Requests `final_url`, and the original URL if that fails: signed CDN
    /// links often expire, while the original URL redirects to a fresh one.
    /// With `attempt`'s credentials, if it has any for that origin: the
    /// state file never holds them, but a restart within one call does.
    /// Failures that may pass are tried again ([`Engine::retrying`]).
    async fn get_with_fallback(
        &self,
        state: &DownloadState,
        range: Option<(u64, Option<u64>, String)>,
        attempt: &mut Attempt,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Response, Error> {
        let final_url = attempt.authorize(&parse_url(&state.final_url)?);
        let url = attempt.authorize(&parse_url(&state.url)?);
        let same = state.final_url == state.url;
        self.retrying(attempt, on_event, || {
            self.get_either(&final_url, &url, same, range.clone())
        })
        .await
    }

    /// One try of [`Engine::get_with_fallback`].
    async fn get_either(
        &self,
        final_url: &Url,
        url: &Url,
        same: bool,
        range: Option<(u64, Option<u64>, String)>,
    ) -> Result<Response, Error> {
        let first = self.get(final_url.clone(), range.clone()).await;
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
        if usable(&first) || same {
            return first;
        }
        self.get(url.clone(), range).await
    }

    /// Sends a request with `send`, and again after a growing wait while it
    /// fails for a reason that may pass: a connection that could not be
    /// made or broke, a timeout, `5xx`, `408 Request Timeout` or `429 Too
    /// Many Requests`. A `429` waits for its `Retry-After` (seconds or an
    /// HTTP date); one that asks for more than
    /// [`crate::retry::LONGEST_RETRY_AFTER`] is not waited for
    /// ([`Error::ServerBusy`]). After `max_failures` failures in a row the
    /// last answer is returned as it is. A certificate that cannot be
    /// trusted, a redirect that is refused or another status (such as
    /// `403` or `404`) is never retried. Same policy as the connections of
    /// a segmented download (M2).
    async fn retrying<F: Future<Output = Result<Response, Error>>>(
        &self,
        attempt: &mut Attempt,
        on_event: &mut (dyn FnMut(Event) + Send),
        mut send: impl FnMut() -> F,
    ) -> Result<Response, Error> {
        loop {
            let result = tokio::select! {
                result = send() => result,
                () = attempt.stop.requested() => return Err(Error::Stopped { state_file: None }),
            };
            let (reason, asked) = match &result {
                Ok(response) if response.status() == StatusCode::TOO_MANY_REQUESTS => {
                    let asked = header_str(response.headers(), header::RETRY_AFTER)
                        .and_then(|v| crate::retry::retry_after(v, SystemTime::now()));
                    if let Some(wait) =
                        asked.filter(|&wait| wait > crate::retry::LONGEST_RETRY_AFTER)
                    {
                        return Err(Error::ServerBusy { wait });
                    }
                    (format!("the server answered {}", response.status()), asked)
                }
                Ok(response)
                    if response.status().is_server_error()
                        || response.status() == StatusCode::REQUEST_TIMEOUT =>
                {
                    (format!("the server answered {}", response.status()), None)
                }
                Ok(_) => return result,
                Err(Error::Http(err)) if may_pass(err) => (error_chain(err), None),
                Err(_) => return result,
            };
            if !attempt.failed(self.options.max_failures) {
                return result;
            }
            let wait = asked.unwrap_or_else(|| crate::retry::backoff(attempt.failures));
            if let Ok(response) = &result
                && response.status() == StatusCode::TOO_MANY_REQUESTS
            {
                on_event(Event::ServerBusy {
                    status: 429,
                    connections: 1,
                    wait: Some(wait),
                });
            }
            drop(result);
            on_event(Event::Retrying { wait, reason });
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = attempt.stop.requested() => return Err(Error::Stopped { state_file: None }),
            }
        }
    }

    /// GETs `url`; with `range`, only bytes `from` to `to` (or to the end)
    /// and only if the file still matches `If-Range`. A Google Drive
    /// file's page is asked for at its download address, and the page
    /// Drive answers a large file with is followed to its link, once (see
    /// [`crate::drive`]). Any other page that comes for Drive's address
    /// (a sign-in page it redirects a private file to, a page saying the
    /// file was downloaded too often) is [`Error::DrivePage`]: never the
    /// file.
    async fn get(
        &self,
        url: Url,
        range: Option<(u64, Option<u64>, String)>,
    ) -> Result<Response, Error> {
        let url = self.drive.direct_link(&url).unwrap_or(url);
        let response = self.send(url.clone(), range.clone()).await?;
        // Only what is asked of Drive's download address is Drive's: a page
        // from anywhere else is the page the user asked for.
        if !(self.drive.is_download(&url) && is_bare_page(&response)) {
            return Ok(response);
        }
        // Drive's own page, at its own address, may lead to the file;
        // one it redirected to elsewhere (a sign-in page) never does.
        if !self.drive.is_download(response.url()) {
            return Err(Error::DrivePage);
        }
        let page_url = response.url().clone();
        let page = read_page(response, crate::drive::MAX_PAGE_BYTES).await?;
        let link = page
            .and_then(|page| self.drive.confirmed_link(&page, &page_url, &url))
            .ok_or(Error::DrivePage)?;
        let response = self.send(link, range).await?;
        if is_bare_page(&response) {
            return Err(Error::DrivePage);
        }
        Ok(response)
    }

    /// Whether `state` is a Google Drive download: its link is a file's
    /// page or its download address, or its file came from there.
    fn is_drive_download(&self, state: &DownloadState) -> bool {
        [&state.url, &state.final_url].iter().any(|u| {
            Url::parse(u)
                .is_ok_and(|u| self.drive.direct_link(&u).is_some() || self.drive.is_download(&u))
        })
    }

    /// One GET of `url`, as it is.
    async fn send(
        &self,
        url: Url,
        range: Option<(u64, Option<u64>, String)>,
    ) -> Result<Response, Error> {
        let mut request = self.browser.apply(&url, self.client.get(url.clone()));
        if let Some((from, to, if_range)) = range {
            let to = to.map(|to| to.to_string()).unwrap_or_default();
            request = request
                .header(header::RANGE, format!("bytes={from}-{to}"))
                .header(header::IF_RANGE, if_range);
        }
        request
            .send()
            .await
            .map_err(|err| Error::Http(without_secrets(err)))
    }
}

/// Why [`Transfer::pump`] stopped before the end of the body.
enum Halt {
    /// The connection broke or closed early: worth trying again.
    Broken(String),
    /// Trying again would not help (a write failed, or the server sent
    /// more than it announced).
    Fatal(String),
    /// The caller asked the download to stop ([`Stop`]).
    Requested,
}

/// An open download: the locked part file and its state.
struct Transfer {
    file: tokio::fs::File,
    state: DownloadState,
    state_file: PathBuf,
    final_path: PathBuf,
    last_save: Instant,
    save_interval: Duration,
    /// A save of this transfer has succeeded: a stop then needs no save of
    /// its own (#132).
    saved: bool,
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
    ///
    /// Meanwhile a resume may have taken the files over: the state file
    /// exists while the first save runs, and a cancelled download releases
    /// the part file's lock at once. Files whose lock someone holds are
    /// left alone, and the lock is held while their names are freed, so a
    /// resume cannot take them in between (found by the cancellation test
    /// of M3, `retry.rs`).
    ///
    /// The names are freed by moving the files aside (emptied), not by removing them
    /// in place: on Windows a file removed while a handle on it is open
    /// stays "delete pending" and keeps its name until the last handle
    /// closes, and a new download of the same name then failed with
    /// `ERROR_ACCESS_DENIED` (#55). That handle was our own lock, closed
    /// only after the removal, or another program's (an antivirus). So the
    /// files are moved aside while locked, our handle is closed, and only
    /// then are the moved files removed.
    fn drop(&mut self) {
        if !self.abandoned.load(Ordering::SeqCst) {
            return;
        }
        let part = part_path(&self.final_path);
        let held = match zajil_platform::fs::open_regular_nofollow(&part, true) {
            Ok(file) => match file.try_lock() {
                Ok(()) => Some(file),
                Err(_) => return, // someone resumed it
            },
            Err(_) => None,
        };
        // Emptied first: the part was given its full size before the first
        // byte, and a moved file left behind (the process dies before
        // removing it) should not keep that space (review of #56).
        if let Some(file) = &held {
            let _ = file.set_len(0);
        }
        let state = state_path(&self.final_path);
        let aside = [state.as_path(), part.as_path()].map(|file| {
            zajil_platform::fs::move_aside(file).map_err(|err| {
                // Not movable (missing, or held without sharing its
                // deletion): removed in place, as before, if it can be.
                if err.kind() != io::ErrorKind::NotFound {
                    let _ = std::fs::remove_file(file);
                }
            })
        });
        drop(held);
        for moved in aside.into_iter().flatten() {
            let _ = std::fs::remove_file(moved);
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

    /// Writes the body of `response` after the current position. `Ok` when
    /// the body ended where it should.
    async fn pump(
        &mut self,
        response: &mut Response,
        stop: &Stop,
        limits: &Limits,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<(), Halt> {
        let total = self.state.content_length;
        loop {
            let chunk = tokio::select! {
                chunk = response.chunk() => chunk,
                () = stop.requested() => return Err(Halt::Requested),
            };
            let chunk = match chunk {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(err) => return Err(Halt::Broken(http_error(err))),
            };
            if total.is_some_and(|t| self.done().saturating_add(chunk.len() as u64) > t) {
                return Err(Halt::Fatal(
                    "the server sent more data than it announced".into(),
                ));
            }
            if let Err(err) = self.file.write_all(&chunk).await {
                return Err(Halt::Fatal(err.to_string()));
            }
            if let Some(cleanup) = self.cleanup.take() {
                cleanup.disarm();
            }
            self.state.segments[0].done += chunk.len() as u64;
            on_event(Event::Progress {
                done: self.done(),
                total,
            });
            // The speed limits: wait before reading more.
            tokio::select! {
                () = limits.take(chunk.len() as u64) => {}
                () = stop.requested() => return Err(Halt::Requested),
            }
            if self.last_save.elapsed() >= self.save_interval
                && let Err(err) = self.checkpoint().await
            {
                return Err(Halt::Fatal(err.to_string()));
            }
        }

        if let Some(total) = total
            && self.done() < total
        {
            return Err(Halt::Broken(format!(
                "the connection closed after {} of {total} bytes",
                self.done()
            )));
        }
        Ok(())
    }

    /// The caller asked to stop: returns only when nothing of the download
    /// runs any more. What is on disk is recorded unless a save already did
    /// (#132): the state file counts only synced bytes. A new download with no
    /// data yet is removed (when `self` is dropped, here: every save was
    /// awaited, so nothing else holds it).
    async fn stopped(mut self) -> Error {
        if self.cleanup.is_some() {
            return Error::Stopped { state_file: None };
        }
        // Owner's decision on #132: after a save, a stop does not save
        // again (the state file counts only synced bytes; about one
        // `save_interval` of progress is downloaded again on resume).
        if !self.saved
            && let Err(err) = self.checkpoint().await
        {
            return err;
        }
        // Waits for anything the file still does, then closes it.
        drop(self.file.into_std().await);
        Error::Stopped {
            state_file: Some(self.state_file),
        }
    }

    /// The part file and the new-download guard, to start over with.
    async fn into_parts(self) -> (PathBuf, File, Option<EmptyDownloadCleanup>) {
        let part = self.file.into_std().await;
        (self.final_path, part, self.cleanup)
    }

    /// Syncs the data, then records it in the state file (ADR-0004 rule 1).
    /// The next save comes `save_interval` after this one starts, so a
    /// slow save does not stretch the time between them (#132).
    async fn checkpoint(&mut self) -> Result<(), Error> {
        self.last_save = Instant::now();
        self.file.flush().await?;
        self.file.sync_data().await?;
        let watch = self.cleanup.as_ref().and_then(EmptyDownloadCleanup::watch);
        save_state(&mut self.state, &self.state_file, watch).await?;
        self.saved = true;
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

    async fn finish(mut self, on_event: &mut (dyn FnMut(Event) + Send)) -> Result<Finished, Error> {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup.disarm(); // an empty file is a complete download
        }
        self.file.flush().await?;
        let file = self.file.into_std().await;
        finish_download(file, self.state, self.final_path, on_event).await
    }
}

/// The origin (`scheme://host[:port]`) the download's bytes came from: its
/// final address, else its link.
fn origin_of(state: &DownloadState) -> Option<String> {
    [&state.final_url, &state.url]
        .into_iter()
        .filter_map(|u| Url::parse(u).ok())
        .map(|u| u.origin())
        .find(|o| o.is_tuple())
        .map(|o| o.ascii_serialization())
}

/// Completes a download whose bytes are all written to `part`: syncs it,
/// records that everything is on disk, renames it to its final name (or a
/// numbered one) and removes the state file.
pub(crate) async fn finish_download(
    part: File,
    mut state: DownloadState,
    final_path: PathBuf,
    on_event: &mut (dyn FnMut(Event) + Send),
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
    // From the internet: the system warns before it runs, and Office opens
    // it in Protected View (security review #84). On the part file, before
    // it takes its name, so the file never has its name without the mark
    // (review of #90): the mark goes with the file through the rename and
    // the link fallbacks. The link's origin only: its path and query may
    // hold a token. A file system that cannot keep the mark (FAT32, exFAT)
    // keeps the file without it.
    if let Some(origin) = origin_of(&state) {
        let marked = part.clone();
        let _ = blocking(move || zajil_platform::origin::mark(&marked, &origin)).await;
    }
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

/// A stop requested before any transfer began, for a download whose files
/// are on disk: it can be resumed from its state file. A new download with
/// no data (`new`) leaves no files.
fn resumable_stop(err: Error, final_path: &Path, new: bool) -> Error {
    match err {
        Error::Stopped { state_file: None } if !new => Error::Stopped {
            state_file: Some(state_path(final_path)),
        },
        other => other,
    }
}

/// The error for a refusal (`status`) of a request made for `state`. A `401`
/// for a download that started with a user name and password, which are no
/// longer in memory, means it needs them again, and the state file never
/// keeps them (ADR-0004, rule 7): say so, rather than a bare 401.
fn refused(
    status: StatusCode,
    state: &DownloadState,
    final_path: &Path,
    attempt: &Attempt,
) -> Error {
    // Only a download that had them: an expired signed link or session
    // also answers 401, and must not ask for a password it never had
    // (review of #29, owner's decision on #28).
    if status == StatusCode::UNAUTHORIZED && attempt.credentials.is_none() && state.had_credentials
    {
        Error::CredentialsNeeded {
            url: state.url.clone(),
            state_file: state_path(final_path),
        }
    } else {
        Error::Status(status)
    }
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

/// Which bodies [`Engine::fetch_page_inner`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadBody {
    Html,
    Any,
}

/// Whether a client follows redirects ([`build_client`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Redirects {
    /// Up to [`Options::max_redirects`], never from https to http: every
    /// download, probe and refresh.
    Follow,
    /// Never: [`Engine::fetch_page`] returns the redirect itself.
    Never,
}

fn build_client(
    options: &Options,
    extra_roots: &[Vec<u8>],
    redirects: Redirects,
) -> Result<Client, Error> {
    // rustls with the `ring` provider, and reqwest's built-in use of the
    // operating system's certificate store and checks
    // (`rustls-platform-verifier`). Nothing here can turn verification off.
    // Installing fails harmlessly if a provider is already installed.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let max_redirects = options.max_redirects;
    let follow = redirect::Policy::custom(move |attempt| {
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
        .redirect(match redirects {
            Redirects::Follow => follow,
            Redirects::Never => redirect::Policy::none(),
        })
        // No `Referer` of our own: after a redirect reqwest would send the
        // redirecting link, with its `?sig=…` token, to the target (review
        // of #74). The browser's page is sent by `FromBrowser` instead.
        .referer(false)
        .connect_timeout(options.connect_timeout)
        .read_timeout(options.idle_timeout)
        // Proxies arrive in M3; until then never pick them up implicitly.
        .no_proxy()
        .user_agent(concat!("Zajil/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(Error::Http)
}

pub(crate) fn parse_url(raw: &str) -> Result<Url, Error> {
    let url = Url::parse(raw).map_err(|err| Error::InvalidUrl {
        url: shown_url(raw),
        reason: err.to_string(),
    })?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        other => Err(Error::UnsupportedScheme(other.to_owned())),
    }
}

/// `raw` (a URL that did not parse) as an error message may show it:
/// without what may be a user name and password ([`without_userinfo`]),
/// and without the query and fragment, which may hold a token (`?sig=…`).
/// Tokens are never printed (owner's decision on #28).
fn shown_url(raw: &str) -> String {
    let mut shown = without_userinfo(raw);
    if let Some(end) = shown.find(['?', '#']) {
        shown.truncate(end);
    }
    shown
}

/// `err` with the secrets its URL may hold removed, so its message never
/// shows them: the user name and password, the query (`?sig=…`) and the
/// fragment.
pub(crate) fn without_secrets(mut err: reqwest::Error) -> reqwest::Error {
    if let Some(url) = err.url_mut() {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
    }
    err
}

/// The message of an HTTP error, without the secrets of its URL.
pub(crate) fn http_error(err: reqwest::Error) -> String {
    error_chain(&without_secrets(err))
}

/// Whether an HTTP error may pass if the request is sent again: a
/// connection that could not be made or broke, or a timeout. A certificate
/// that cannot be trusted does not pass by itself and is never retried;
/// nor is a refused redirect.
fn may_pass(err: &reqwest::Error) -> bool {
    if err.is_builder() || err.is_redirect() || is_tls_error(err) {
        return false;
    }
    err.is_timeout() || err.is_connect() || err.is_request() || err.is_body()
}

/// Whether a TLS error (such as an untrusted certificate) is behind `err`.
/// `io::Error` skips its own inner error in `source()`, and the TLS error
/// sits in an `io::Error` inside another one, so go into each `io::Error`
/// instead of asking for its source.
fn is_tls_error(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut next = Some(err);
    while let Some(err) = next {
        if err.is::<rustls::Error>() {
            return true;
        }
        next = match err.downcast_ref::<io::Error>() {
            Some(io) => io
                .get_ref()
                .map(|inner| inner as &(dyn std::error::Error + 'static)),
            None => err.source(),
        };
    }
    false
}

/// `raw` (a URL that did not parse) with what may be a user name and
/// password hidden, so an error message never shows them: everything up to
/// the last `@`, except a leading `http:` or `https:` and its slashes. It
/// does not rely on `://`, which a mistyped URL may lack (`http:/u:p@h`) or
/// have again later (`?next=https://…`); and anything before the `@` that
/// is not that scheme may be a token (`TOKEN:x-oauth-basic@h`, `http//…`).
/// A password may hold `/`, `#` or `?`, so the last `@` is the only safe
/// end; an `@` later in the URL hides a little too much, never too little.
fn without_userinfo(raw: &str) -> String {
    let Some(at) = raw.rfind('@') else {
        return raw.to_owned();
    };
    let head = &raw[..at];
    let scheme = ["https:", "http:"].into_iter().find(|scheme| {
        head.get(..scheme.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(scheme))
    });
    let keep = match scheme {
        Some(scheme) => {
            let rest = &head[scheme.len()..];
            scheme.len() + rest.len() - rest.trim_start_matches('/').len()
        }
        None => 0,
    };
    format!("{}***{}", &raw[..keep], &raw[at..])
}

/// The download's final path from its state file's path, once the path is
/// one Zajil would make: the state file and everything next to it are
/// untrusted (they may come from another machine).
fn checked_final_path(state_file: &Path) -> Result<PathBuf, Error> {
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
    Ok(final_path)
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

/// The name a response's file gets: from its `Content-Disposition` or its
/// address, cleaned, with the extension of its `Content-Type` when it has
/// none (`view` sent as `text/html` is `view.html`).
fn response_name(response: &Response) -> String {
    let name = suggested_name(
        response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .map(|v| v.as_bytes()),
        response.url(),
    );
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    with_type_extension(name, content_type)
}

/// A web page sent as a page (`200`, `text/html` or
/// `application/xhtml+xml`, without `Content-Disposition`), not as a file:
/// a file, an `.html` one too, comes with `Content-Disposition` from Drive.
fn is_bare_page(response: &Response) -> bool {
    let page = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|t| is_page_type(&t.trim().to_ascii_lowercase()));
    response.status() == StatusCode::OK
        && page
        && !response.headers().contains_key(header::CONTENT_DISPOSITION)
}

/// The text of a page of at most `limit` bytes; `None` when it is larger
/// or not text.
async fn read_page(mut response: Response, limit: usize) -> Result<Option<String>, Error> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| Error::Http(without_secrets(err)))?
    {
        if body.len().saturating_add(chunk.len()) > limit {
            return Ok(None);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8(body).ok())
}

/// The first state for a whole-file response. `had_credentials`: the URL
/// the user gave had a user name or password (never saved themselves).
fn state_from_response(url: String, had_credentials: bool, response: &Response) -> DownloadState {
    let headers = response.headers();
    let accept_ranges = header_str(headers, header::ACCEPT_RANGES)
        .is_some_and(|v| v.split(',').any(|u| u.trim().eq_ignore_ascii_case("bytes")));
    let mut state = DownloadState::new(
        url,
        without_credentials(response.url()),
        validator_from(headers),
        response.content_length(),
        accept_ranges,
    );
    state.had_credentials = had_credentials;
    state
}

/// The `If-Range` value RFC 9110 §13.1.5 allows, or `None` when there is no
/// strong validator and the download must start over rather than risk
/// mixing two versions of the file:
/// - a strong `ETag` (a weak one, `W/"…"`, must never be sent);
/// - only when there is **no** `ETag` at all, `Last-Modified`, and only if
///   it is strong (§8.8.2.2): at least one second before the `Date` of the
///   same response. A file changed twice within that second would otherwise
///   look unchanged.
pub(crate) fn if_range_value(validator: &Validator) -> Option<String> {
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
    fn an_invalid_url_never_shows_its_password() {
        // Review of #29: the whole URL, password included, was in the error.
        for raw in [
            "http://u:S3cret@127.0.0.1:99999/f.bin",
            "http://u:pa#S3cret@127.0.0.1:1/f.bin",
            "http://u:pa?S3cret@h:1/f",
            "http://u:pa/S3cret@h:99999/f",
            "u:S3cret@h:99999/f",
        ] {
            let message = parse_url(raw).unwrap_err().to_string();
            assert!(!message.contains("S3cret"), "{message}");
        }
        assert_eq!(
            without_userinfo("http://u:S3cret@h:99999/f"),
            "http://***@h:99999/f"
        );
        // Review of #29: a `://` only later in the URL, or none at all.
        for raw in [
            "http:/u:S3cret@h:99999/f?next=https://x",
            "http:/u:S3cret@h:99999/f#https://x",
            "http:u:S3cret@h:99999/f",
            // Third review of #29: a token as the user name, and no scheme
            // or a mistyped one.
            "S3cret_TOKEN:x-oauth-basic@h:99999/f",
            "http//S3cret_TOKEN:x-oauth-basic@h:99999/f",
            "https;//S3cret_TOKEN:x@h:99999/f",
            "HTTPS://S3cret:x@h:99999/f",
        ] {
            assert!(!without_userinfo(raw).contains("S3cret"), "{raw}");
            if let Err(err) = parse_url(raw) {
                assert!(!err.to_string().contains("S3cret"), "{err}");
            }
        }
        assert_eq!(without_userinfo("http://h:99999/f"), "http://h:99999/f");
        assert_eq!(without_userinfo("http:/u:p@h:1/f"), "http:/***@h:1/f");
        assert_eq!(without_userinfo("T:x@h:1/f"), "***@h:1/f");
    }

    #[test]
    fn an_invalid_url_never_shows_a_token() {
        // Review of #29 (owner's decision on #28): `?sig=…` was shown.
        for raw in [
            "http://h:99999/f.bin?sig=S3cret",
            "http://h:99999/f.bin#S3cret",
            "http://u:p@h:99999/f.bin?a=1&sig=S3cret#x",
        ] {
            let message = parse_url(raw).unwrap_err().to_string();
            assert!(!message.contains("S3cret"), "{message}");
            assert!(message.contains("h:99999/f.bin"), "{message}");
        }
        // An `@` in the query hides more than needed, never less.
        let message = parse_url("http://h:99999/f.bin?next=http://u:p@x/?S3cret")
            .unwrap_err()
            .to_string();
        assert!(!message.contains("S3cret"), "{message}");
        assert_eq!(shown_url("http://h:1/f?sig=x#y"), "http://h:1/f");
        assert_eq!(shown_url("http://h:1/f"), "http://h:1/f");
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
