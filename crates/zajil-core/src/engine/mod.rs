//! Downloading one file over HTTP or HTTPS.
//!
//! Life cycle (docs/ARCHITECTURE.md §3.1):
//! 1. One GET request; its headers are the probe (size, `Accept-Ranges`,
//!    `ETag`, `Last-Modified`, name).
//! 2. If the size is known, it must fit in the free space of the folder
//!    before any file is created.
//! 3. A free name is reserved in the download folder: `<name>.zajil-part`
//!    (locked while we use it) and `<name>.zajil-state`, and the part file is
//!    given its size (`zajil_platform::fs::allocate`: a sparse file, nothing
//!    is reserved).
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

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use reqwest::{Client, StatusCode, Url};

use crate::limit::{Clock, Limits, SpeedLimit, SystemClock};
use crate::naming::ReserveError;
use crate::sanitize::{ConfineError, sanitize_file_name};
use crate::state::StateError;

mod download;
mod files;
mod http;
mod probe;
mod resume;
mod transfer;

pub(crate) use files::{blocking, save_state};
use http::{Redirects, build_client, error_chain};
pub(crate) use http::{changed_from, http_error, if_range_value, parse_content_range, parse_url};
pub use probe::{FetchedPage, Probe, is_page_type};
pub(crate) use transfer::{EmptyDownloadCleanup, NewDownload, finish_download};

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
    /// Every request goes through this proxy (`http://host:port`, checked
    /// by [`crate::settings::check_proxy`]); `None`: straight to the
    /// server. TLS is still checked end to end through it.
    pub proxy: Option<String>,
    /// At most this many bytes per range request: a connection asks again
    /// for the next part of its segment (`None`: the rest of its segment in
    /// one request). YouTube slows a request for much more than 10 MiB, as
    /// yt-dlp knows (its `http_chunk_size`).
    pub request_size: Option<u64>,
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
            proxy: None,
            request_size: None,
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

/// What [`Engine::holding`] checks a new download's names and size with.
#[derive(Clone)]
struct Hold(HoldCheck);

/// Whether to hold a new download, from the name the server gives, the
/// name it is saved under ([`Engine::named`]; the same when it has none)
/// and the size the server announced (`None` when it announced none).
pub type HoldCheck = Arc<dyn Fn(&str, &str, Option<u64>) -> bool + Send + Sync>;

impl std::fmt::Debug for Hold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Hold")
    }
}

/// What tests can ask of the engine (`test-support` only, never in a
/// shipped binary).
#[cfg(feature = "test-support")]
pub mod engine_test_hooks {
    use std::sync::atomic::AtomicBool;

    /// The next start of a download stops right after its part file was
    /// emptied and allocated, as a crash there would (#163). Set it from a
    /// test file of its own: it is one switch for the whole process.
    pub static CRASH_AFTER_ALLOCATING: AtomicBool = AtomicBool::new(false);
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

    /// This engine, with its browser's headers, sent as `agent` (a
    /// `User-Agent`) instead of Zajil.
    pub fn with_user_agent(&self, agent: Option<&str>) -> Engine {
        self.with_browser((*self.browser).clone().with_user_agent(agent))
    }

    /// The site of `url` (#170): `None` when it has no limit; else the
    /// site's identity (the same number for every URL of that site) and its
    /// limit. A queue runner starts no more of its own downloads on a site
    /// than its limit, so one that would only wait takes no place in its
    /// queue; the downloads it starts share the site, as each waits for
    /// its own slot before any extra connection takes one.
    pub fn site_room(&self, url: &str) -> Option<(usize, usize)> {
        let url = Url::parse(url).ok()?;
        let (counter, limit) = self.sites.entry_for(&url)?;
        Some((Arc::as_ptr(&counter) as usize, limit))
    }

    /// The site limits of this engine and its clones, changed in place by
    /// [`crate::sites::SiteLimits::set`] (#165).
    pub fn site_limits(&self) -> &crate::sites::SiteLimits {
        &self.sites
    }

    /// This engine, counting the connections to each site in `sites` (the
    /// settings file's `site_connections`) across all its downloads and
    /// those of its clones made after this.
    pub fn with_site_limits(&self, sites: crate::sites::SiteLimits) -> Engine {
        let mut engine = self.clone();
        engine.sites = Arc::new(sites);
        engine
    }

    /// This engine, holding a new download whose names and size make `hold`
    /// true (the server's name, the one it is saved under, the size the
    /// server announced): it stops with [`Error::Held`] once the server has
    /// answered, before anything is created. For a file from a page that
    /// can run code: the probe's name is not enough, the download's own
    /// answer may name another file (review of #90). For the grabber's
    /// limits: a file announced larger than what is left is refused before
    /// a byte of it is written (#196).
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

    /// This engine, asking for at most `bytes` per range request (see
    /// [`Options::request_size`]).
    pub fn with_request_size(&self, bytes: Option<u64>) -> Engine {
        let mut engine = self.clone();
        engine.options.request_size = bytes.map(|b| b.max(1));
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
}
