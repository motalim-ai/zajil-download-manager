//! Zajil's engine for the Android app (ADR-0012): what its Kotlin calls,
//! through UniFFI. The logic stays in `zajil-core`; this only starts, pauses
//! and reports downloads, each on the engine's own state file (ADR-0004), so
//! a download killed with the app resumes from where it stopped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use zajil_core::{Engine, Error, Event, Options, Stop};

#[cfg(target_os = "android")]
mod tls;

uniffi::setup_scaffolding!();

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
    Other,
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

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum EngineError {
    #[error("{detail}")]
    Setup { detail: String },
}

/// The app's downloads. One for the whole process.
#[derive(uniffi::Object)]
pub struct Downloads {
    runtime: tokio::runtime::Runtime,
    engine: Engine,
    listener: Arc<dyn Listener>,
    running: Arc<Mutex<HashMap<i64, Stop>>>,
}

enum Job {
    New { url: String, dir: PathBuf },
    Resume { state_file: PathBuf },
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
        self.run(
            id,
            Job::New {
                url,
                dir: dir.into(),
            },
        );
    }

    /// Resumes download `id` from its state file. Nothing happens if it is
    /// already running.
    pub fn resume(&self, id: i64, state_file: String) {
        self.run(
            id,
            Job::Resume {
                state_file: state_file.into(),
            },
        );
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
        Ok(Arc::new(Downloads {
            runtime,
            engine: engine.map_err(|err| setup(&err))?,
            listener,
            running: Arc::default(),
        }))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Stop>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn run(&self, id: i64, job: Job) {
        let stop = Stop::new();
        {
            let mut running = self.lock();
            if running.contains_key(&id) {
                return;
            }
            running.insert(id, stop.clone());
        }
        let engine = self.engine.clone();
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
