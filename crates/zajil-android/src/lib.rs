//! Zajil's engine for the Android app (ADR-0012): what its Kotlin calls,
//! through UniFFI. The logic stays in `zajil-core`; this only starts, pauses
//! and reports downloads, each on the engine's own state file (ADR-0004), so
//! a download killed with the app resumes from where it stopped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use zajil_core::limit::SpeedLimit;
use zajil_core::{Engine, Error, Event, FromBrowser, Options, Stop};

pub mod lists;
#[cfg(target_os = "android")]
mod tls;
pub mod tools;
pub mod update;

uniffi::setup_scaffolding!();

/// How long [`Downloads::probe`] tries a link before it gives up.
const PROBE_FOR: Duration = Duration::from_secs(20);

/// Progress reaches Kotlin at most this often (a notification is updated
/// from it, and Android drops updates that come faster).
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// Why a download stopped without finishing, for the app to say in the
/// user's language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Problem {
    /// No connection, or the connection kept failing.
    Network,
    /// The server refused the request.
    Server,
    /// The device has no room for the file.
    NotEnoughSpace,
    /// The certificate of an HTTPS server was not valid.
    Certificate,
    /// The link is not one Zajil can download.
    BadLink,
    /// The link, or a redirect, leads to a site on Zajil's list of
    /// harmful sites ([`HostCheck`]): nothing was asked of it.
    Refused,
    Other,
}

/// The app's lists of harmful sites (malware, phishing), asked for each
/// host a download goes to: the link's and every redirect's. Called on the
/// engine's threads; it must answer at once, from memory.
#[uniffi::export(with_foreign)]
pub trait HostCheck: Send + Sync {
    /// Whether `host` (lower case) is refused.
    fn refused(&self, host: String) -> bool;
}

/// What a running download reports. Called on the engine's threads.
#[uniffi::export(with_foreign)]
pub trait Listener: Send + Sync {
    /// Data starts to flow, from byte `from`; from now on the download
    /// resumes from `state_file`.
    fn started(&self, id: i64, file: String, state_file: String, total: Option<u64>, from: u64);
    /// Bytes on disk so far.
    fn progress(&self, id: i64, done: u64, total: Option<u64>);
    /// The download is complete at `file`.
    fn finished(&self, id: i64, file: String, bytes: u64);
    /// Paused as asked: nothing of it runs any more. `state_file` resumes it
    /// (none if no data had been written).
    fn paused(&self, id: i64, state_file: Option<String>);
    /// Stopped by `problem`. `detail` is the engine's own words, without the
    /// link's query or credentials. Resumable from `state_file` if there is one.
    fn failed(&self, id: i64, problem: Problem, detail: String, state_file: Option<String>);
}

/// How one download is made, beyond the engine's defaults. A video's
/// streams take YouTube's own (as `zajil-core`'s video_fast does on the
/// desktop): yt-dlp's User-Agent, more connections, and requests of at most
/// 10 MiB, since YouTube slows a request for more.
#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct Tuning {
    /// The file's name, instead of the server's.
    pub name: Option<String>,
    pub user_agent: Option<String>,
    pub connections: Option<u32>,
    pub request_size: Option<u64>,
    /// The in-app browser's cookies (a `Cookie` header) for
    /// [`Tuning::cookie_url`], sent only to that address's origin, never to
    /// a host a redirect leads to (`zajil_core::FromBrowser`). In memory
    /// only: never in the state file.
    #[uniffi(default = None)]
    pub cookies: Option<String>,
    #[uniffi(default = None)]
    pub cookie_url: Option<String>,
    /// The page the link was found on: sent as the browsers do
    /// (`strict-origin-when-cross-origin`).
    #[uniffi(default = None)]
    pub referrer: Option<String>,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum EngineError {
    #[error("{detail}")]
    Setup { detail: String },
    /// [`Downloads::probe`] could not reach the file. `detail` is the
    /// engine's own words, without the link's query or credentials.
    #[error("{detail}")]
    Probe { problem: Problem, detail: String },
}

/// What a link leads to, before it is downloaded: for the sheet that asks
/// the user (the desktop's add dialog, M4).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LinkInfo {
    /// The file's name, as a download would give it (sanitized).
    pub name: String,
    /// Its size, if the server said.
    pub size: Option<u64>,
    /// It can be split over several connections and resumed later.
    pub resumable: bool,
    /// Its media type, lower case, without parameters.
    pub content_type: Option<String>,
    /// The server answered with a web page, not a file (a sign-in page).
    pub is_page: bool,
}

/// The app's downloads. One for the whole process.
#[derive(uniffi::Object)]
pub struct Downloads {
    runtime: tokio::runtime::Runtime,
    engine: Engine,
    listener: Arc<dyn Listener>,
    running: Arc<Mutex<HashMap<i64, Stop>>>,
    /// The app's speed limit, shared by all its downloads.
    limit: SpeedLimit,
    /// The app's lists of harmful sites, if it has set them.
    guard: Arc<Mutex<Option<zajil_core::browser::HostGuard>>>,
}

enum Job {
    New { url: String, dir: PathBuf },
    Resume { state_file: PathBuf },
}

impl Tuning {
    fn apply(&self, engine: &Engine, guard: Option<zajil_core::browser::HostGuard>) -> Engine {
        let mut engine = engine
            .named(self.name.as_deref())
            .with_user_agent(self.user_agent.as_deref())
            .with_request_size(self.request_size);
        if self.cookies.is_some() || self.referrer.is_some() || guard.is_some() {
            engine = engine.with_browser(self.browser().with_host_guard(guard));
        }
        if let Some(connections) = self.connections {
            engine = engine.with_connections(usize::try_from(connections).unwrap_or(usize::MAX));
        }
        engine
    }

    /// What the browser adds to the requests: its cookies for their own
    /// origin, and the page.
    fn browser(&self) -> FromBrowser {
        FromBrowser::new(
            self.cookie_url.as_deref().unwrap_or_default(),
            self.cookies.as_deref(),
            self.referrer.as_deref(),
        )
        .with_user_agent(self.user_agent.as_deref())
    }
}

#[uniffi::export]
impl Downloads {
    #[uniffi::constructor]
    pub fn new(listener: Arc<dyn Listener>) -> Result<Arc<Self>, EngineError> {
        Self::with_engine(listener, Engine::new(Options::default()))
    }

    /// Starts downloading `url` into the folder `dir`. Nothing happens if
    /// download `id` is already running.
    pub fn start(&self, id: i64, url: String, dir: String) {
        self.start_tuned(id, url, dir, Tuning::default());
    }

    /// Like [`Downloads::start`], made as [`Tuning`] says.
    pub fn start_tuned(&self, id: i64, url: String, dir: String, tuning: Tuning) {
        self.run(
            id,
            Job::New {
                url,
                dir: dir.into(),
            },
            &tuning,
        );
    }

    /// Resumes download `id` from its state file. Nothing happens if it is
    /// already running.
    pub fn resume(&self, id: i64, state_file: String) {
        self.resume_tuned(id, state_file, Tuning::default());
    }

    /// Like [`Downloads::resume`], made as [`Tuning`] says (its name is
    /// the state file's).
    pub fn resume_tuned(&self, id: i64, state_file: String, tuning: Tuning) {
        self.run(
            id,
            Job::Resume {
                state_file: state_file.into(),
            },
            &tuning,
        );
    }

    /// Gives a stopped download a new link to the same file (a video
    /// server's links expire after some hours): the engine checks that it is
    /// the same file, and what is on disk is kept. Blocks until done; call
    /// it off the main thread, and never while the download runs.
    pub fn refresh_link(&self, state_file: String, url: String) -> Result<(), EngineError> {
        self.runtime
            .block_on(self.engine.refresh_link(Path::new(&state_file), &url))
            .map_err(|err| EngineError::Setup {
                detail: err.to_string(),
            })
    }

    /// Asks download `id` to stop; [`Listener::paused`] follows once
    /// nothing of it runs any more.
    pub fn pause(&self, id: i64) {
        if let Some(stop) = self.lock().get(&id) {
            stop.request();
        }
    }

    pub fn is_running(&self, id: i64) -> bool {
        self.lock().contains_key(&id)
    }

    /// How many downloads are running.
    pub fn running(&self) -> u32 {
        u32::try_from(self.lock().len()).unwrap_or(u32::MAX)
    }

    /// The speed limit of all downloads together, in bytes per second
    /// (`None` or 0: none). Running downloads follow it at once.
    pub fn set_speed_limit(&self, bytes_per_second: Option<u64>) {
        self.limit.set_rate(bytes_per_second);
    }

    /// What `url` leads to, without downloading it: the first response's
    /// headers only. Blocks for at most 20 seconds; call it off the main
    /// thread.
    /// The app's lists of harmful sites: from now on no download, nor
    /// probe, asks a host `check` refuses, nor follows a redirect to one.
    /// `None` checks nothing.
    pub fn set_host_check(&self, check: Option<Arc<dyn HostCheck>>) {
        let guard = check.map(|check| -> zajil_core::browser::HostGuard {
            Arc::new(move |host: &str| check.refused(host.to_owned()))
        });
        *self.guard.lock().unwrap_or_else(PoisonError::into_inner) = guard;
    }

    pub fn probe(&self, url: String) -> Result<LinkInfo, EngineError> {
        self.probe_tuned(url, Tuning::default())
    }

    /// Like [`Downloads::probe`], made as [`Tuning`] says (the in-app
    /// browser's cookies and page).
    pub fn probe_tuned(&self, url: String, tuning: Tuning) -> Result<LinkInfo, EngineError> {
        let engine = tuning.apply(&self.engine, self.guard());
        let found = self
            .runtime
            .block_on(async { tokio::time::timeout(PROBE_FOR, engine.probe(&url)).await });
        match found {
            Ok(Ok(probe)) => Ok(LinkInfo {
                is_page: probe.is_page(),
                name: probe.name,
                size: probe.size,
                resumable: probe.resumable,
                content_type: probe.content_type,
            }),
            Ok(Err(err)) => Err(EngineError::Probe {
                problem: describe(&err).0,
                detail: err.to_string(),
            }),
            Err(_) => Err(EngineError::Probe {
                problem: Problem::Network,
                detail: "the server did not answer in time".to_owned(),
            }),
        }
    }
}

impl Downloads {
    /// Tests only: an engine that also trusts the test CA
    /// (`Engine::with_test_root`, which exists only with `test-support`).
    #[doc(hidden)]
    pub fn with_engine(
        listener: Arc<dyn Listener>,
        engine: Result<Engine, Error>,
    ) -> Result<Arc<Self>, EngineError> {
        let setup = |err: &dyn std::fmt::Display| EngineError::Setup {
            detail: err.to_string(),
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("zajil-engine")
            .enable_all()
            .build()
            .map_err(|err| setup(&err))?;
        let engine = engine.map_err(|err| setup(&err))?;
        let limit = SpeedLimit::new(None, engine.clock());
        Ok(Arc::new(Downloads {
            runtime,
            engine: engine.with_speed_limit(Some(limit.clone())),
            listener,
            running: Arc::default(),
            limit,
            guard: Arc::default(),
        }))
    }

    fn guard(&self) -> Option<zajil_core::browser::HostGuard> {
        self.guard
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Stop>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn run(&self, id: i64, job: Job, tuning: &Tuning) {
        let stop = Stop::new();
        {
            let mut running = self.lock();
            if running.contains_key(&id) {
                return;
            }
            running.insert(id, stop.clone());
        }
        let engine = tuning.apply(&self.engine, self.guard());
        let listener = self.listener.clone();
        let running = self.running.clone();
        self.runtime.spawn(async move {
            let reporter = listener.clone();
            let mut last: Option<Instant> = None;
            let mut on_event = move |event: Event| match event {
                Event::Started {
                    file,
                    state_file,
                    total,
                    from,
                } => reporter.started(id, text(&file), text(&state_file), total, from),
                Event::Progress { done, total } => {
                    let now = Instant::now();
                    if last.is_none_or(|at| now.duration_since(at) >= PROGRESS_EVERY) {
                        last = Some(now);
                        reporter.progress(id, done, total);
                    }
                }
                _ => {}
            };
            let result = match &job {
                Job::New { url, dir } => {
                    engine.download_until(url, dir, &stop, &mut on_event).await
                }
                Job::Resume { state_file } => {
                    engine.resume_until(state_file, &stop, &mut on_event).await
                }
            };
            // Not running any more before Kotlin hears of it, so that it can
            // resume at once.
            running
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
            match result {
                Ok(done) => listener.finished(id, text(&done.file), done.bytes),
                Err(Error::Stopped { state_file }) => {
                    listener.paused(id, state_file.as_deref().map(text));
                }
                Err(err) => {
                    let (problem, state_file) = describe(&err);
                    listener.failed(id, problem, err.to_string(), state_file.map(text));
                }
            }
        });
    }
}

/// The problem behind `err`, and the state file it can resume from.
fn describe(err: &Error) -> (Problem, Option<&Path>) {
    match err {
        Error::Interrupted { state_file, .. } => (Problem::Network, Some(state_file)),
        Error::Http(source) if is_certificate(source) => (Problem::Certificate, None),
        Error::Http(_) => (Problem::Network, None),
        Error::Status(_) | Error::ServerBusy { .. } => (Problem::Server, None),
        Error::NotEnoughSpace { .. } => (Problem::NotEnoughSpace, None),
        Error::InvalidUrl { .. } | Error::UnsupportedScheme(_) => (Problem::BadLink, None),
        Error::Refused { .. } => (Problem::Refused, None),
        _ => (Problem::Other, None),
    }
}

/// Whether a request failed on the server's certificate: rustls says so,
/// somewhere down the chain (an `io::Error` hides its inner error from
/// `source()`, so it is opened by hand).
fn is_certificate(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut next = Some(err);
    while let Some(err) = next {
        if let Some(rustls::Error::InvalidCertificate(_)) = err.downcast_ref::<rustls::Error>() {
            return true;
        }
        next = match err.downcast_ref::<std::io::Error>() {
            Some(io) => io
                .get_ref()
                .map(|inner| inner as &(dyn std::error::Error + 'static)),
            None => err.source(),
        };
    }
    false
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
