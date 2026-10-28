//! A local HTTP and HTTPS server for Zajil's tests. Never shipped.
//!
//! It serves files from memory and can simulate the hard cases from
//! `AGENTS.md`: a server that ignores `Range`, a connection cut in the middle,
//! a file that changes on the server (new `ETag`), slow or stalled transfers,
//! `429` with `Retry-After`, `403` after some requests, redirects and
//! dangerous names in `Content-Disposition`, and bodies held at a
//! [`Gate`] until the test (or a number of responses at once) opens it.
//! Every request is logged with the
//! number of body bytes the server sent, so tests can check what was asked
//! for twice, with the connection it came on and how many responses for the
//! same file were being sent at that moment.
//!
//! The server runs on its own thread and runtime, so it works from both
//! synchronous and asynchronous tests, and stops when dropped.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::convert::Infallible;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::thread::JoinHandle;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::channel::{Channel, Sender};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty};
use hyper::body::{Frame, Incoming, SizeHint};
use hyper::header::{self, HeaderValue};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_rustls::TlsAcceptor;

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type Body = BoxBody<Bytes, BoxError>;

const CHUNK: usize = 64 * 1024;

/// `Last-Modified` used unless a test sets another.
pub const DEFAULT_LAST_MODIFIED: &str = "Wed, 21 Oct 2026 07:28:00 GMT";

/// A failure returned instead of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub status: u16,
    /// Seconds for the `Retry-After` header.
    pub retry_after: Option<u64>,
}

/// Holds response bodies ([`FileSpec::hold_after`]) until it opens: when
/// the test calls [`Gate::open`], or, for [`Gate::when_together`], when
/// that many bodies of its file are being sent at once. Once open, it stays
/// open. Clones share it.
#[derive(Debug, Clone)]
pub struct Gate {
    open: Arc<tokio::sync::watch::Sender<bool>>,
    together: Option<usize>,
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

impl Gate {
    /// A gate that only [`Gate::open`] opens.
    pub fn new() -> Self {
        Gate {
            open: Arc::new(tokio::sync::watch::Sender::new(false)),
            together: None,
        }
    }

    /// A gate that the server opens once `bodies` bodies of the file it
    /// holds are being sent at the same moment: what a test of downloads
    /// at once needs, as each one waits until the others have started.
    pub fn when_together(bodies: usize) -> Self {
        Gate {
            together: Some(bodies),
            ..Self::new()
        }
    }

    pub fn open(&self) {
        self.open.send_replace(true);
    }

    pub fn is_open(&self) -> bool {
        *self.open.borrow()
    }

    async fn opened(&self) {
        let mut open = self.open.subscribe();
        let _ = open.wait_for(|open| *open).await;
    }
}

/// A file served at one path, and how the server misbehaves for it.
#[derive(Debug, Clone)]
pub struct FileSpec {
    body: Bytes,
    announced_length: Option<u64>,
    etag: Option<String>,
    etag_per_connection: Option<(String, String)>,
    authorization: Option<String>,
    last_modified: Option<String>,
    date: Option<String>,
    ranges: bool,
    honor_ranges: bool,
    ignore_if_range: bool,
    send_length: bool,
    content_disposition: Option<String>,
    content_type: Option<String>,
    cut_after: Option<(u64, usize)>,
    stall_after: Option<u64>,
    holds: Vec<(u64, Gate)>,
    bytes_per_sec: Option<u64>,
    slow_at: Option<(u64, u64)>,
    fail_first: Vec<Failure>,
    forbid_after: Option<usize>,
    busy_beyond: Option<(usize, Failure)>,
    fail_ranges: Option<Failure>,
    replaced_after: Option<(usize, Box<FileSpec>)>,
    answer_after: Option<Duration>,
}

impl FileSpec {
    /// A well-behaved file: `Content-Length`, `Accept-Ranges: bytes`, a
    /// fresh strong `ETag` and a fixed `Last-Modified`.
    pub fn new(body: impl Into<Bytes>) -> Self {
        static NEXT_VERSION: AtomicU64 = AtomicU64::new(1);
        let version = NEXT_VERSION.fetch_add(1, Ordering::Relaxed);
        FileSpec {
            body: body.into(),
            announced_length: None,
            etag: Some(format!("\"v{version}\"")),
            etag_per_connection: None,
            authorization: None,
            last_modified: Some(DEFAULT_LAST_MODIFIED.to_owned()),
            date: None,
            ranges: true,
            honor_ranges: true,
            ignore_if_range: false,
            send_length: true,
            content_disposition: None,
            content_type: None,
            cut_after: None,
            stall_after: None,
            holds: Vec::new(),
            bytes_per_sec: None,
            slow_at: None,
            fail_first: Vec::new(),
            forbid_after: None,
            busy_beyond: None,
            fail_ranges: None,
            replaced_after: None,
            answer_after: None,
        }
    }

    /// The server ignores `Range` and always answers `200` with the whole
    /// file, without `Accept-Ranges`.
    pub fn without_ranges(mut self) -> Self {
        self.ranges = false;
        self.honor_ranges = false;
        self
    }

    /// The server announces `Accept-Ranges: bytes` but ignores `Range` and
    /// always answers `200` with the whole file.
    pub fn ignore_ranges(mut self) -> Self {
        self.honor_ranges = false;
        self
    }

    /// Two machines behind one name, each with its own `ETag` for the same
    /// bytes (as Apache or nginx compute them from the inode or the time):
    /// odd-numbered connections see `odd`, even-numbered ones `even`, and
    /// `If-Range` is checked against the one the connection sees.
    pub fn etag_per_connection(mut self, odd: &str, even: &str) -> Self {
        self.etag_per_connection = Some((odd.to_owned(), even.to_owned()));
        self
    }

    /// Every GET without exactly this `Authorization` header gets
    /// `401 Unauthorized` (for Basic, `"Basic "` and the base64 of
    /// `user:password`).
    pub fn require_authorization(mut self, value: &str) -> Self {
        self.authorization = Some(value.to_owned());
        self
    }

    /// The server serves `Range` requests even when `If-Range` does not
    /// match (it ignores `If-Range`, as some servers do).
    pub fn ignore_if_range(mut self) -> Self {
        self.ignore_if_range = true;
        self
    }

    /// `200` responses announce this `Content-Length` whatever the body is:
    /// a server that lies about the size (for example a huge one).
    pub fn announce_length(mut self, length: u64) -> Self {
        self.announced_length = Some(length);
        self
    }

    pub fn etag(mut self, etag: Option<&str>) -> Self {
        self.etag = etag.map(str::to_owned);
        self
    }

    /// The `Last-Modified` header, or `None` for none. The default is
    /// [`DEFAULT_LAST_MODIFIED`] for every file, so a test that replaces a
    /// file without an `ETag` must set a new value here to show a change,
    /// or the same value to simulate a change within the same second.
    pub fn last_modified(mut self, value: Option<&str>) -> Self {
        self.last_modified = value.map(str::to_owned);
        self
    }

    /// The `Date` header. By default the server sends the current time.
    /// Tests of RFC 9110 §8.8.2.2 set it close to `Last-Modified`.
    pub fn date(mut self, value: &str) -> Self {
        self.date = Some(value.to_owned());
        self
    }

    /// No `Content-Length`: the body is sent chunked.
    pub fn without_length(mut self) -> Self {
        self.send_length = false;
        self
    }

    /// Raw value of the `Content-Disposition` header.
    pub fn content_disposition(mut self, value: &str) -> Self {
        self.content_disposition = Some(value.to_owned());
        self
    }

    /// Raw value of the `Content-Type` header (`application/octet-stream`
    /// by default).
    pub fn content_type(mut self, value: &str) -> Self {
        self.content_type = Some(value.to_owned());
        self
    }

    /// The first `times` GET responses are cut after `bytes` body bytes.
    pub fn cut_after(mut self, bytes: u64, times: usize) -> Self {
        self.cut_after = Some((bytes, times));
        self
    }

    /// Every GET response stops sending after `bytes` body bytes and keeps
    /// the connection open.
    pub fn stall_after(mut self, bytes: u64) -> Self {
        self.stall_after = Some(bytes);
        self
    }

    /// Every GET response sends `bytes` body bytes, then holds the rest
    /// (keeping the connection open) until `gate` opens; once open, it holds
    /// nothing. Several holds can be set, each at its own place, so a test
    /// can stop every connection at known points one after the other.
    pub fn hold_after(mut self, bytes: u64, gate: &Gate) -> Self {
        self.holds.push((bytes, gate.clone()));
        self.holds.sort_by_key(|(at, _)| *at);
        self
    }

    /// Every request for it is answered only after `wait`: a server slow to
    /// answer, so a window shows what it shows while it waits.
    pub fn answer_after(mut self, wait: Duration) -> Self {
        self.answer_after = Some(wait);
        self
    }

    /// Limits each response to about `bytes_per_sec`.
    pub fn throttle(mut self, bytes_per_sec: u64) -> Self {
        self.bytes_per_sec = Some(bytes_per_sec.max(1));
        self
    }

    /// A response that starts at byte `start` of the file (a range request
    /// for that byte, or the whole file for `0`) is limited to about
    /// `bytes_per_sec`: one slow connection among fast ones.
    pub fn slow_at(mut self, start: u64, bytes_per_sec: u64) -> Self {
        self.slow_at = Some((start, bytes_per_sec.max(1)));
        self
    }

    /// The first GET requests get these failures, in order.
    pub fn fail_first(mut self, failures: Vec<Failure>) -> Self {
        self.fail_first = failures;
        self
    }

    /// GET requests after the first `requests` get `403 Forbidden`.
    pub fn forbid_after(mut self, requests: usize) -> Self {
        self.forbid_after = Some(requests);
        self
    }

    /// While `connections` bodies of this file are being sent, any other GET
    /// gets `403 Forbidden`: a server that allows that many connections at
    /// once.
    pub fn max_concurrent(mut self, connections: usize) -> Self {
        self.busy_beyond = Some((
            connections,
            Failure {
                status: 403,
                retry_after: None,
            },
        ));
        self
    }

    /// Like [`FileSpec::max_concurrent`], but the extra requests get
    /// `429 Too Many Requests` with `Retry-After: retry_after` seconds.
    pub fn too_many_beyond(mut self, connections: usize, retry_after: u64) -> Self {
        self.busy_beyond = Some((
            connections,
            Failure {
                status: 429,
                retry_after: Some(retry_after),
            },
        ));
        self
    }

    /// Every GET with a `Range` header gets `failure`; plain GETs are served.
    pub fn fail_range_requests(mut self, failure: Failure) -> Self {
        self.fail_ranges = Some(failure);
        self
    }

    /// After `gets` GET requests, the file is replaced by `next` (a new
    /// version): the file changes on the server in the middle of a
    /// download, at a known moment.
    pub fn replaced_after(mut self, gets: usize, next: FileSpec) -> Self {
        self.replaced_after = Some((gets, Box::new(next)));
        self
    }

    pub fn body(&self) -> &Bytes {
        &self.body
    }

    pub fn current_etag(&self) -> Option<&str> {
        self.etag.as_deref()
    }
}

/// One request as the server saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRecord {
    pub method: String,
    pub path: String,
    pub range: Option<String>,
    pub if_range: Option<String>,
    pub status: u16,
    /// Body bytes handed to the connection: an upper bound of what the
    /// client received, counted when each chunk is handed over. A cut
    /// response counts only what was sent before the cut. Not a speed
    /// measurement.
    ///
    /// **Written, not received**: a cut response can count what the
    /// connection still held and threw away, hundreds of KiB when the
    /// reader is slow (64 KiB in the diagnostic of #50: 3,145,728 counted,
    /// 3,080,192 read by a raw client). Equal to what arrived only for a
    /// response that completed.
    pub bytes_sent: u64,
    /// The TLS version of the connection (`"TLSv1_3"`), or `None` for HTTP.
    pub tls_version: Option<String>,
    /// The `Authorization` header, if any.
    pub authorization: Option<String>,
    /// The `Cookie` header, if any.
    pub cookie: Option<String>,
    /// The `Referer` header, if any.
    pub referer: Option<String>,
    /// Number of the TCP connection, counted from 1 in the order the server
    /// accepted them. Requests on the same connection share it (keep-alive).
    pub connection: u64,
    /// Bodies of this file being sent at the moment this request arrived,
    /// this one included (0 for a response without a body).
    pub concurrent: usize,
    /// When the request arrived, from the start of the server.
    pub at: Duration,
}

struct LogEntry {
    record: RequestRecord,
    bytes_sent: Arc<AtomicU64>,
}

struct Served {
    spec: FileSpec,
    gets: usize,
    sending: Arc<Sending>,
}

/// A failure response, with `Retry-After` if it has one.
fn failure_response(failure: &Failure) -> Response<Body> {
    let mut builder = Response::builder().status(failure.status);
    if let Some(secs) = failure.retry_after {
        builder = builder.header(header::RETRY_AFTER, secs.to_string());
    }
    builder.body(empty()).unwrap()
}

/// Bodies of one file being sent right now, and the most at once.
#[derive(Default)]
struct Sending {
    now: AtomicUsize,
    most: AtomicUsize,
}

impl Sending {
    /// Counts a body until the returned guard is dropped, which hyper does
    /// when the body is finished or the client goes away.
    fn start(self: &Arc<Self>) -> (usize, SendingGuard) {
        let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
        self.most.fetch_max(now, Ordering::SeqCst);
        (now, SendingGuard(self.clone()))
    }
}

struct SendingGuard(Arc<Sending>);

impl Drop for SendingGuard {
    fn drop(&mut self) {
        self.0.now.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A response body that is counted in [`Sending`] while it exists.
struct Counted {
    inner: Body,
    _guard: SendingGuard,
}

impl hyper::body::Body for Counted {
    type Data = Bytes;
    type Error = BoxError;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, BoxError>>> {
        Pin::new(&mut self.inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

struct Shared {
    started: std::time::Instant,
    /// The start of every path this server serves (`/<random>`): a request
    /// without it comes from someone else, such as a process left by
    /// another test that connects to this port number (#50).
    prefix: String,
    /// Those requests (`GET /f.bin`): answered 404, never counted.
    strays: Vec<String>,
    /// Every connection: who it came from and how it ended, for the report
    /// printed when a test fails (#50).
    connections: Vec<ConnectionLog>,
    files: HashMap<String, Served>,
    redirects: HashMap<String, (String, u16)>,
    log: Vec<LogEntry>,
    /// `/robots.txt` at the root is served as `/robots.txt` under the
    /// prefix ([`TestServer::robots_txt_at_root`]).
    robots_at_root: bool,
}

/// A running test server. Stops when dropped.
pub struct TestServer {
    addr: SocketAddr,
    base: String,
    shared: Arc<Mutex<Shared>>,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    /// Plain HTTP on `127.0.0.1` and a free port.
    pub fn http() -> Self {
        Self::http_on((Ipv4Addr::LOCALHOST, 0).into()).expect("bind 127.0.0.1")
    }

    /// Plain HTTP on the given address (for example `[::1]:0`).
    pub fn http_on(addr: SocketAddr) -> io::Result<Self> {
        Self::start(addr, None, "http", None)
    }

    /// HTTPS on `127.0.0.1` with the certificate described by `tls`. The
    /// URL uses `host`, which may differ from the certificate's names to
    /// test a certificate for the wrong host.
    pub fn https(tls: Tls, host: &str) -> Self {
        let (cert, key) = tls.issue();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("TLS versions")
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .expect("server certificate");
        let acceptor = TlsAcceptor::from(Arc::new(config));
        Self::start(
            (Ipv4Addr::LOCALHOST, 0).into(),
            Some(acceptor),
            "https",
            Some(host),
        )
        .expect("bind 127.0.0.1")
    }

    /// HTTPS with a fresh self-signed certificate for `names`, which no
    /// client trusts.
    pub fn https_self_signed(names: &[&str], host: &str) -> Self {
        Self::https(Tls::self_signed(names), host)
    }

    fn start(
        addr: SocketAddr,
        tls: Option<TlsAcceptor>,
        scheme: &str,
        host: Option<&str>,
    ) -> io::Result<Self> {
        let shared = Arc::new(Mutex::new(Shared {
            started: std::time::Instant::now(),
            prefix: random_prefix(),
            strays: Vec::new(),
            robots_at_root: false,
            connections: Vec::new(),
            files: HashMap::new(),
            redirects: HashMap::new(),
            log: Vec::new(),
        }));
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (bound_tx, bound_rx) = std::sync::mpsc::channel();
        let server_shared = shared.clone();

        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("test server runtime");
            runtime.block_on(serve(addr, tls, server_shared, shutdown_rx, bound_tx));
            runtime.shutdown_timeout(Duration::from_millis(200));
        });

        let addr = match bound_rx.recv() {
            Ok(Ok(addr)) => addr,
            Ok(Err(err)) => {
                let _ = thread.join();
                return Err(err);
            }
            Err(_) => return Err(io::Error::other("test server thread exited")),
        };
        let authority = match host {
            Some(host) => format!("{host}:{}", addr.port()),
            None => addr.to_string(),
        };
        let prefix = shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .prefix
            .clone();
        Ok(TestServer {
            addr,
            base: format!("{scheme}://{authority}{prefix}"),
            shared,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Full URL for `path` (which starts with `/`), under this server's
    /// own prefix.
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// The path a raw request must use for `path` (with this server's
    /// prefix), for tests that write HTTP by hand.
    pub fn path(&self, path: &str) -> String {
        format!("{}{path}", self.lock().prefix)
    }

    /// Everything this server saw, for a failed test (printed by `Drop`
    /// when the test panics): each request in order, with its range,
    /// status and bytes; each connection, with who opened it and how it
    /// ended; and the stray requests.
    pub fn report(&self) -> String {
        // One look at the log, so requests and connections agree (review
        // of #52: a request could come between two looks).
        let shared = self.lock();
        let mut out = format!("test server {} saw:\n", self.base);
        for (n, entry) in shared.log.iter().enumerate() {
            let r = &entry.record;
            out.push_str(&format!(
                "  request {}: {} {} range={:?} if_range={:?} status={} bytes_written={} conn={} at={:?}\n",
                n + 1,
                r.method,
                r.path,
                r.range,
                r.if_range,
                r.status,
                entry.bytes_sent.load(Ordering::SeqCst),
                r.connection,
                r.at
            ));
        }
        for c in &shared.connections {
            let ended = match &c.ended {
                Some((at, how)) => format!("ended at {at:?}: {how}"),
                // The client may already see it closed: its end is written
                // down just after the socket closes.
                None => "still open (or closing)".to_owned(),
            };
            out.push_str(&format!(
                "  connection {} from {} at {:?}, {ended}\n",
                c.number, c.peer, c.accepted
            ));
        }
        for s in &shared.strays {
            out.push_str(&format!("  stray request (404, not counted): {s}\n"));
        }
        if shared.log.is_empty() && shared.connections.is_empty() && shared.strays.is_empty() {
            out.push_str("  (nothing)\n");
        }
        out
    }

    /// Requests that did not use this server's prefix, oldest first: they
    /// were answered 404 and are not in [`TestServer::requests`].
    pub fn strays(&self) -> Vec<String> {
        self.lock().strays.clone()
    }

    /// From now on, `/robots.txt` at the root of this server (where a
    /// crawler asks for it, without the prefix) is served as `/robots.txt`
    /// under the prefix: what [`put`](Self::put) or
    /// [`redirect`](Self::redirect) gave that path, else 404. It is then a
    /// request like any other, in [`requests`](Self::requests), and not a
    /// stray. For the site grabber's tests (M7).
    pub fn robots_txt_at_root(&self) {
        self.lock().robots_at_root = true;
    }

    /// Serves `spec` at `path`, replacing any previous file there. A new
    /// [`FileSpec`] has a new `ETag`, so this simulates a changed file.
    /// A `path` with a `?query` serves requests with exactly that query
    /// (their records have it in their `path`); others go to the `path`
    /// without it. The same for [`TestServer::redirect`].
    pub fn put(&self, path: &str, spec: FileSpec) {
        self.lock().files.insert(
            path.to_owned(),
            Served {
                spec,
                gets: 0,
                sending: Arc::default(),
            },
        );
    }

    /// The most bodies of the file at `path` that were sent at the same
    /// time, since it was [`put`](Self::put).
    pub fn most_concurrent(&self, path: &str) -> usize {
        self.lock()
            .files
            .get(path)
            .map_or(0, |served| served.sending.most.load(Ordering::SeqCst))
    }

    /// Answers requests for `from` with a redirect to `to` (a path or URL).
    pub fn redirect(&self, from: &str, to: &str, status: u16) {
        self.lock()
            .redirects
            .insert(from.to_owned(), (to.to_owned(), status));
    }

    /// Every request so far, oldest first.
    pub fn requests(&self) -> Vec<RequestRecord> {
        self.lock()
            .log
            .iter()
            .map(|entry| RequestRecord {
                bytes_sent: entry.bytes_sent.load(Ordering::SeqCst),
                ..entry.record.clone()
            })
            .collect()
    }

    /// GET requests for `path` so far, oldest first.
    pub fn gets(&self, path: &str) -> Vec<RequestRecord> {
        self.requests()
            .into_iter()
            .filter(|r| r.method == "GET" && r.path == path)
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // A test failed with this server alive: say everything it saw, so a
        // failure that comes once in many runs leaves its evidence the first
        // time (#50). Shown by the test harness under the failed test.
        if std::thread::panicking() {
            // `eprintln!`, not a write to `std::io::stderr()`: only the
            // macros go to the harness's capture, which shows them under the
            // failed test (a direct write went past it, review of #52). The
            // cost: with `--nocapture` and a stderr that is a closed pipe the
            // macro panics, and a panic during a panic aborts the binary.
            // Neither happens in CI.
            eprintln!("{}", self.report());
        }
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

async fn serve(
    addr: SocketAddr,
    tls: Option<TlsAcceptor>,
    shared: Arc<Mutex<Shared>>,
    mut shutdown: oneshot::Receiver<()>,
    bound: std::sync::mpsc::Sender<io::Result<SocketAddr>>,
) {
    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) => {
            let _ = bound.send(Err(err));
            return;
        }
    };
    let _ = bound.send(listener.local_addr());

    let mut connections = 0u64;
    loop {
        let stream = tokio::select! {
            _ = &mut shutdown => return,
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(_) => continue,
            },
        };
        let (stream, peer) = stream;
        let shared = shared.clone();
        let tls = tls.clone();
        connections += 1;
        let connection = connections;
        {
            let mut log = shared.lock().unwrap_or_else(|e| e.into_inner());
            let at = log.started.elapsed();
            log.connections.push(ConnectionLog {
                number: connection,
                peer: peer.to_string(),
                accepted: at,
                ended: None,
            });
        }
        let shared_for_log = shared.clone();
        tokio::spawn(async move {
            let service = |tls_version: Option<String>| {
                service_fn(move |req| {
                    let shared = shared.clone();
                    let tls_version = tls_version.clone();
                    async move {
                        if let Some(wait) = answer_after(&req, &shared) {
                            tokio::time::sleep(wait).await;
                        }
                        Ok::<_, Infallible>(handle(req, &shared, tls_version, connection))
                    }
                })
            };
            let http = hyper::server::conn::http1::Builder::new();
            let how = match tls {
                Some(acceptor) => match acceptor.accept(stream).await {
                    Ok(stream) => {
                        let version = stream
                            .get_ref()
                            .1
                            .protocol_version()
                            .map(|v| format!("{v:?}"));
                        ended_how(
                            http.serve_connection(TokioIo::new(stream), service(version))
                                .await,
                        )
                    }
                    // A client that rejects our certificate ends here.
                    Err(err) => format!("TLS handshake failed: {err}"),
                },
                None => ended_how(
                    http.serve_connection(TokioIo::new(stream), service(None))
                        .await,
                ),
            };
            let mut log = shared_for_log.lock().unwrap_or_else(|e| e.into_inner());
            let at = log.started.elapsed();
            if let Some(entry) = log.connections.iter_mut().find(|c| c.number == connection) {
                entry.ended = Some((at, how));
            }
        });
    }
}

/// How long the file a request is for waits before it is answered
/// ([`FileSpec::answer_after`]).
fn answer_after(req: &Request<Incoming>, shared: &Mutex<Shared>) -> Option<Duration> {
    let shared = shared.lock().unwrap_or_else(|e| e.into_inner());
    let path = req.uri().path().strip_prefix(shared.prefix.as_str())?;
    shared.files.get(path)?.spec.answer_after
}

fn handle(
    req: Request<Incoming>,
    shared: &Mutex<Shared>,
    tls_version: Option<String>,
    connection: u64,
) -> Response<Body> {
    let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
    let header = |name| {
        req.headers()
            .get(name)
            .and_then(|v: &HeaderValue| v.to_str().ok())
            .map(str::to_owned)
    };
    // Only requests under this server's prefix are its own; any other is
    // answered 404 and counted nowhere, so it cannot take the place of a
    // test's request (in `cut_after`, `fail_first`, the request log…).
    let robots = shared.robots_at_root && req.uri().path() == "/robots.txt";
    let Some(mut path) = req
        .uri()
        .path()
        .strip_prefix(shared.prefix.as_str())
        .filter(|rest| rest.starts_with('/'))
        .map(str::to_owned)
        .or_else(|| robots.then(|| "/robots.txt".to_owned()))
    else {
        let stray = format!("{} {}", req.method(), req.uri().path());
        shared.strays.push(stray);
        return status_only(StatusCode::NOT_FOUND);
    };
    // A file put at `path?query` answers that query only (Google Drive's
    // page and file differ by their query alone).
    if let Some(query) = req.uri().query() {
        let with_query = format!("{path}?{query}");
        if shared.files.contains_key(&with_query) || shared.redirects.contains_key(&with_query) {
            path = with_query;
        }
    }
    let is_head = req.method() == Method::HEAD;
    let bytes_sent = Arc::new(AtomicU64::new(0));
    let mut record = RequestRecord {
        method: req.method().to_string(),
        path: path.clone(),
        range: header(header::RANGE),
        if_range: header(header::IF_RANGE),
        authorization: header(header::AUTHORIZATION),
        cookie: header(header::COOKIE),
        referer: header(header::REFERER),
        status: 0,
        bytes_sent: 0,
        tls_version,
        connection,
        concurrent: 0,
        at: shared.started.elapsed(),
    };

    let response = if let Some((to, status)) = shared.redirects.get(&path) {
        // A path on this server stays under its prefix.
        let to = if to.starts_with('/') {
            format!("{}{to}", shared.prefix)
        } else {
            to.clone()
        };
        Response::builder()
            .status(*status)
            .header(header::LOCATION, to)
            .body(empty())
            .unwrap()
    } else if let Some(served) = shared.files.get_mut(&path) {
        if let Some((after, next)) = &served.spec.replaced_after
            && !is_head
            && served.gets >= *after
        {
            served.spec = (**next).clone();
            served.gets = 0;
        }
        let get_index = served.gets;
        if !is_head {
            served.gets += 1;
        }
        let refused = if is_head {
            None
        } else if let Some(failure) = served
            .spec
            .fail_ranges
            .as_ref()
            .filter(|_| record.range.is_some())
        {
            Some(failure_response(failure))
        } else {
            served
                .spec
                .busy_beyond
                .as_ref()
                .filter(|(connections, _)| {
                    served.sending.now.load(Ordering::SeqCst) >= *connections
                })
                .map(|(_, failure)| failure_response(failure))
        };
        let response = if let Some(refused) = refused {
            refused
        } else {
            respond(
                &served.spec,
                &record,
                is_head,
                get_index,
                bytes_sent.clone(),
            )
        };
        // Count the bodies of this file while they are being sent.
        if !is_head
            && matches!(
                response.status(),
                StatusCode::OK | StatusCode::PARTIAL_CONTENT
            )
        {
            let (now, guard) = served.sending.start();
            record.concurrent = now;
            for (_, gate) in &served.spec.holds {
                if gate.together.is_some_and(|bodies| now >= bodies) {
                    gate.open();
                }
            }
            response.map(|inner| {
                Counted {
                    inner,
                    _guard: guard,
                }
                .boxed()
            })
        } else {
            response
        }
    } else {
        status_only(StatusCode::NOT_FOUND)
    };

    record.status = response.status().as_u16();
    shared.log.push(LogEntry { record, bytes_sent });
    response
}

fn respond(
    spec: &FileSpec,
    record: &RequestRecord,
    is_head: bool,
    get_index: usize,
    bytes_sent: Arc<AtomicU64>,
) -> Response<Body> {
    let per_connection;
    let spec = match &spec.etag_per_connection {
        Some((odd, even)) => {
            let etag = if record.connection % 2 == 1 {
                odd
            } else {
                even
            };
            per_connection = FileSpec {
                etag: Some(etag.clone()),
                ..spec.clone()
            };
            &per_connection
        }
        None => spec,
    };
    if !is_head && spec.authorization.is_some() && record.authorization != spec.authorization {
        return status_only(StatusCode::UNAUTHORIZED);
    }
    if !is_head {
        if spec.forbid_after.is_some_and(|n| get_index >= n) {
            return status_only(StatusCode::FORBIDDEN);
        }
        if let Some(failure) = spec.fail_first.get(get_index) {
            return failure_response(failure);
        }
    }

    let total = spec.body.len() as u64;
    let range = if spec.honor_ranges
        && (spec.ignore_if_range || if_range_matches(spec, record.if_range.as_deref()))
    {
        record.range.as_deref().map(|r| parse_range(r, total))
    } else {
        None
    };

    let content_type = spec
        .content_type
        .as_deref()
        .unwrap_or("application/octet-stream");
    let mut builder = Response::builder().header(header::CONTENT_TYPE, content_type);
    if spec.ranges {
        builder = builder.header(header::ACCEPT_RANGES, "bytes");
    }
    if let Some(etag) = &spec.etag {
        builder = builder.header(header::ETAG, etag);
    }
    if let Some(date) = &spec.last_modified {
        builder = builder.header(header::LAST_MODIFIED, date);
    }
    if let Some(date) = &spec.date {
        // hyper adds the current time only when no Date header is set.
        builder = builder.header(header::DATE, date);
    }
    if let Some(cd) = &spec.content_disposition {
        builder = builder.header(header::CONTENT_DISPOSITION, cd);
    }

    let (status, start, end) = match range {
        None => (StatusCode::OK, 0, total),
        Some(Some((start, last))) => {
            builder = builder.header(
                header::CONTENT_RANGE,
                format!("bytes {start}-{last}/{total}"),
            );
            (StatusCode::PARTIAL_CONTENT, start, last + 1)
        }
        Some(None) => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{total}"))
                .body(empty())
                .unwrap();
        }
    };
    if spec.send_length {
        let length = match spec.announced_length {
            Some(announced) if status == StatusCode::OK => announced,
            _ => end - start,
        };
        builder = builder.header(header::CONTENT_LENGTH, length);
    }
    builder = builder.status(status);
    if is_head {
        return builder.body(empty()).unwrap();
    }

    let body = spec.body.slice(start as usize..end as usize);
    let cut = spec
        .cut_after
        .filter(|&(_, times)| get_index < times)
        .map(|(bytes, _)| bytes);
    let (sender, channel) = Channel::<Bytes, BoxError>::new(1);
    // A response that announced more than it has stalls at the end of what
    // it has, keeping the connection open. Closing it would make Windows
    // send a reset that can drop the headers before the client reads them.
    let short = spec
        .announced_length
        .filter(|&announced| status == StatusCode::OK && announced > body.len() as u64)
        .map(|_| body.len() as u64);
    let stall = [spec.stall_after, short].into_iter().flatten().min();
    let throttle = match spec.slow_at {
        Some((slow, bytes_per_sec)) if slow == start => Some(bytes_per_sec),
        _ => spec.bytes_per_sec,
    };
    let holds = spec.holds.clone();
    tokio::spawn(stream_body(
        sender, body, cut, stall, holds, throttle, bytes_sent,
    ));
    builder.body(channel.boxed()).unwrap()
}

async fn stream_body(
    mut sender: Sender<Bytes, BoxError>,
    body: Bytes,
    cut: Option<u64>,
    stall: Option<u64>,
    holds: Vec<(u64, Gate)>,
    bytes_per_sec: Option<u64>,
    bytes_sent: Arc<AtomicU64>,
) {
    let limit = [cut, stall]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(u64::MAX)
        .min(body.len() as u64) as usize;
    let chunk = match bytes_per_sec {
        Some(bps) => (bps as usize / 20).clamp(1, CHUNK),
        None => CHUNK,
    };

    let mut offset = 0;
    while offset < limit {
        let mut next = (offset + chunk).min(limit);
        // Held at each place reached until its gate opens, and never sent
        // past the next one.
        for (at, gate) in &holds {
            let at = usize::try_from(*at).unwrap_or(usize::MAX);
            if offset >= at {
                gate.opened().await;
            } else {
                next = next.min(at);
                break;
            }
        }
        let sent = next - offset;
        // Count before handing the chunk over, so the count is complete by
        // the time the client has read everything.
        bytes_sent.fetch_add(sent as u64, Ordering::SeqCst);
        if sender.send_data(body.slice(offset..next)).await.is_err() {
            bytes_sent.fetch_sub(sent as u64, Ordering::SeqCst);
            return; // client went away
        }
        offset = next;
        if let Some(bps) = bytes_per_sec {
            tokio::time::sleep(Duration::from_secs_f64(sent as f64 / bps as f64)).await;
        }
    }

    if offset == body.len() && stall.is_none_or(|stall| stall as usize > offset) {
        return; // complete
    }
    if cut.is_some_and(|c| c as usize == offset) {
        sender.abort(Box::new(io::Error::other("cut by test server")));
        return;
    }
    // Stalled: keep the connection open and send nothing.
    let _keep = sender;
    std::future::pending::<()>().await;
}

/// RFC 9110 §13.1.5: a strong `ETag` that matches exactly, or the exact
/// `Last-Modified` date. Weak validators never match.
fn if_range_matches(spec: &FileSpec, if_range: Option<&str>) -> bool {
    match if_range {
        None => true,
        Some(value) if value.starts_with("W/") => false,
        Some(value) if value.starts_with('"') => spec.etag.as_deref() == Some(value),
        Some(value) => spec.last_modified.as_deref() == Some(value),
    }
}

/// Parses a single `bytes=` range. `Some(None)` means unsatisfiable.
fn parse_range(value: &str, total: u64) -> Option<(u64, u64)> {
    let spec = value.strip_prefix("bytes=")?.trim();
    if spec.contains(',') {
        return None;
    }
    let (first, last) = spec.split_once('-')?;
    let (start, end) = if first.is_empty() {
        let suffix: u64 = last.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        (total.saturating_sub(suffix), total.checked_sub(1)?)
    } else {
        let start: u64 = first.parse().ok()?;
        let end = if last.is_empty() {
            total.checked_sub(1)?
        } else {
            last.parse::<u64>().ok()?.min(total.checked_sub(1)?)
        };
        (start, end)
    };
    (start <= end && start < total).then_some((start, end))
}

fn empty() -> Body {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}

fn status_only(status: StatusCode) -> Response<Body> {
    Response::builder().status(status).body(empty()).unwrap()
}

/// The certificate an HTTPS test server presents.
#[derive(Debug, Clone)]
pub struct Tls {
    names: Vec<String>,
    kind: CertKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CertKind {
    /// Signed by the test CA and valid now.
    Trusted,
    /// Signed by the test CA, but expired yesterday.
    Expired,
    /// Signed by itself; nobody trusts it.
    SelfSigned,
}

impl Tls {
    /// Signed by [`test_ca_der`] and valid now. A client trusts it only if
    /// the test adds that CA.
    pub fn trusted(names: &[&str]) -> Self {
        Self::new(names, CertKind::Trusted)
    }

    /// Signed by the test CA, but no longer valid.
    pub fn expired(names: &[&str]) -> Self {
        Self::new(names, CertKind::Expired)
    }

    /// Self-signed; no client trusts it.
    pub fn self_signed(names: &[&str]) -> Self {
        Self::new(names, CertKind::SelfSigned)
    }

    fn new(names: &[&str], kind: CertKind) -> Self {
        Tls {
            names: names.iter().map(|n| (*n).to_owned()).collect(),
            kind,
        }
    }

    fn issue(
        &self,
    ) -> (
        rustls::pki_types::CertificateDer<'static>,
        rustls::pki_types::PrivateKeyDer<'static>,
    ) {
        let key = rcgen::KeyPair::generate().expect("server key");
        let mut params = rcgen::CertificateParams::new(self.names.clone()).expect("names");
        params.distinguished_name.push(
            rcgen::DnType::CommonName,
            self.names.first().cloned().unwrap_or_default(),
        );
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        let now = rcgen::date_time_ymd(1970, 1, 1) + since_epoch();
        let day = Duration::from_secs(24 * 3600);
        (params.not_before, params.not_after) = match self.kind {
            CertKind::Expired => (now - 30 * day, now - day),
            // Short validity: macOS refuses server certificates valid for
            // more than 825 days, even under a custom root.
            _ => (now - day, now + 30 * day),
        };
        let cert = match self.kind {
            CertKind::SelfSigned => params.self_signed(&key),
            _ => params.signed_by(&key, &test_ca().issuer),
        }
        .expect("sign certificate");
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into());
        (cert.der().clone(), key)
    }
}

/// The certificate authority of this test run, created on first use and
/// kept only in memory: no certificate or key is ever stored in the
/// repository.
struct TestCa {
    issuer: rcgen::CertifiedIssuer<'static, rcgen::KeyPair>,
}

fn test_ca() -> &'static TestCa {
    static CA: std::sync::OnceLock<TestCa> = std::sync::OnceLock::new();
    CA.get_or_init(|| {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).expect("params");
        params.distinguished_name.push(
            rcgen::DnType::CommonName,
            "Zajil temporary test CA (do not trust)",
        );
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
            rcgen::KeyUsagePurpose::DigitalSignature,
        ];
        let now = rcgen::date_time_ymd(1970, 1, 1) + since_epoch();
        let day = Duration::from_secs(24 * 3600);
        params.not_before = now - 2 * day;
        params.not_after = now + 60 * day;
        let key = rcgen::KeyPair::generate().expect("CA key");
        TestCa {
            issuer: rcgen::CertifiedIssuer::self_signed(params, key).expect("CA certificate"),
        }
    })
}

/// DER of this test run's CA certificate, for tests that must trust
/// [`Tls::trusted`] servers.
pub fn test_ca_der() -> Vec<u8> {
    test_ca().issuer.der().to_vec()
}

/// Time since 1970, to build rcgen dates from `date_time_ymd(1970, 1, 1)`.
fn since_epoch() -> Duration {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after 1970")
}

/// Deterministic pseudo-random bytes (SplitMix64), fast enough for large
/// test files.
pub fn random_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        out.extend_from_slice(&(z ^ (z >> 31)).to_le_bytes());
    }
    out.truncate(len);
    out
}

/// Lowercase hex SHA-256 of `data`.
pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// One connection, for [`TestServer::report`].
#[derive(Debug, Clone)]
struct ConnectionLog {
    number: u64,
    peer: String,
    accepted: Duration,
    /// When and how it ended, if it has.
    ended: Option<(Duration, String)>,
}

/// How a connection ended, in words: normally (the client closed it, or
/// the server stopped), or with hyper's error and its causes (a reset from
/// the client shows its I/O error kind, such as `ConnectionReset`).
fn ended_how(result: Result<(), hyper::Error>) -> String {
    match result {
        Ok(()) => "normally".to_owned(),
        Err(err) => {
            let mut text = format!(
                "{err} (incomplete message: {}, body write aborted: {})",
                err.is_incomplete_message(),
                err.is_body_write_aborted()
            );
            let mut source = std::error::Error::source(&err);
            while let Some(cause) = source {
                match cause.downcast_ref::<io::Error>() {
                    Some(io) => text.push_str(&format!(" <- {:?}: {io}", io.kind())),
                    None => text.push_str(&format!(" <- {cause}")),
                }
                source = cause.source();
            }
            text
        }
    }
}

/// `/` and 16 hex digits, different for every server.
fn random_prefix() -> String {
    use std::hash::{BuildHasher, Hasher};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut hasher = std::hash::RandomState::new().build_hasher();
    hasher.write_u64(NEXT.fetch_add(1, Ordering::Relaxed));
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    format!("/{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ranges() {
        assert_eq!(parse_range("bytes=0-9", 100), Some((0, 9)));
        assert_eq!(parse_range("bytes=90-", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=50-500", 100), Some((50, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=5-1", 100), None);
        assert_eq!(parse_range("bytes=0-1,5-6", 100), None);
        assert_eq!(parse_range("items=0-1", 100), None);
        assert_eq!(parse_range("bytes=0-", 0), None);
    }

    #[test]
    fn if_range_uses_strong_comparison() {
        let spec = FileSpec::new(&b"x"[..]).etag(Some("\"a\""));
        assert!(if_range_matches(&spec, None));
        assert!(if_range_matches(&spec, Some("\"a\"")));
        assert!(!if_range_matches(&spec, Some("\"b\"")));
        assert!(!if_range_matches(&spec, Some("W/\"a\"")));
        assert!(if_range_matches(&spec, Some(DEFAULT_LAST_MODIFIED)));
        assert!(!if_range_matches(
            &spec,
            Some("Thu, 01 Jan 1970 00:00:00 GMT")
        ));
    }

    #[test]
    fn random_bytes_are_deterministic() {
        assert_eq!(random_bytes(1000, 7), random_bytes(1000, 7));
        assert_ne!(random_bytes(1000, 7), random_bytes(1000, 8));
        assert_eq!(random_bytes(13, 1).len(), 13);
    }

    #[test]
    fn new_specs_get_new_etags() {
        let a = FileSpec::new(&b"x"[..]);
        let b = FileSpec::new(&b"x"[..]);
        assert_ne!(a.current_etag(), b.current_etag());
    }
}
