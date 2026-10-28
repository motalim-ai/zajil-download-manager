//! Downloading one file over several connections (M2).
//!
//! The file is cut into contiguous segments, one per connection to start
//! with. Every connection ("worker") owns one segment at a time and writes
//! what it receives at that segment's offset, through the one locked handle
//! of the `.zajil-part` file (`zajil_platform::fs::write_at`).
//!
//! Rules that keep the data right:
//! - A byte range is written only by the worker that owns its segment, and
//!   a worker reserves the bytes it is about to write (`pending`) before
//!   writing them. Nobody takes over a segment that has bytes written or
//!   reserved.
//! - A response is used only if it is exactly what was asked for: a `206`
//!   whose `Content-Range` is the requested range of a file of the same size
//!   and version (RFC 9110 §14.4, §13.1.5). A `200` to a range request means
//!   the file changed (the whole download starts over) or the server
//!   ignores ranges (no more range requests).
//! - `done` counts only bytes that `write_at` returned for. A checkpoint
//!   copies the state, syncs the file, then saves the copy (ADR-0004 rule
//!   1), so the state file never counts bytes that are not on disk.
//!
//! A connection that reaches the end of its segment keeps reading into the
//! next segment if that one has nothing written or reserved yet. So the
//! first response (a plain `200` with the whole file) is never wasted, and a
//! server that ignores ranges is still read to the end on one connection.
//!
//! **Dynamic splitting** (docs/ARCHITECTURE.md §3.2): a connection with no
//! free segment left takes the second half of what remains of the segment
//! with the longest time left (remaining bytes ÷ its connection's speed),
//! if each half is at least the minimum segment size. The old connection
//! stops where the new segment begins. Every split is reported as
//! [`Event::Split`] and saved with the next checkpoint.
//!
//! **Retries and busy servers** (`retry`): a connection that fails (cut,
//! timeout, `5xx`, `408`) gives its segment back with a wait that grows
//! each time, with a random part; after `max_failures` failures in a row
//! (by default [`retry::MAX_FAILURES`]) without progress the segment is given up and the download stops,
//! resumable. A `429` makes every connection wait for `Retry-After`; a `429`
//! or a `403` while other connections work means the server limits
//! connections, so the limit drops to the connections it accepted.
//!
//! **A full disk** (nothing is reserved when a download starts): a write
//! or a save that fails is not tried again. Every connection stops at once,
//! and the download is resumable from the state file saved last, which
//! counts only bytes on disk.

use std::fs::File;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use reqwest::header;
use reqwest::{Client, Response, StatusCode, Url};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

use crate::engine::{
    EmptyDownloadCleanup, Error, Event, Finished, NewDownload, RestartReason, Stop, blocking,
    changed_from, finish_download, http_error, parse_content_range, save_state,
};
use crate::limit::Limits;
use crate::retry;
use crate::state::{DownloadState, Segment, Validator, state_path};

/// Most connections for one download.
pub const MAX_CONNECTIONS: usize = 32;

/// Connections for one download unless the user chooses otherwise.
pub const DEFAULT_CONNECTIONS: usize = 8;

/// Bytes a connection collects before writing them.
const WRITE_BATCH: usize = 256 * 1024;

/// A connection writes what it has after this long even if the batch is not
/// full, so slow connections still show progress and get saved.
const WRITE_AFTER: Duration = Duration::from_millis(200);

/// A connection's speed is trusted only after it has worked this long.
const SPEED_AFTER: Duration = Duration::from_millis(500);

/// A connection that has sent nothing for this long counts as stalled: its
/// segment is the first one to split.
const STALLED_AFTER: Duration = Duration::from_secs(1);

/// How often a connection with nothing to do looks again for a segment to
/// split.
const LOOK_AGAIN: Duration = Duration::from_millis(100);

/// How long a connection waits before it looks again for a free slot of a
/// limited site (`crate::sites`), which another download may give back.
const SITE_LOOK_AGAIN: Duration = Duration::from_millis(250);

/// The segments for a new download of `len` bytes over at most
/// `connections` connections, none smaller than `min_segment` bytes; `None`
/// when one connection is all it needs.
pub(crate) fn plan(len: u64, connections: usize, min_segment: u64) -> Option<Vec<Segment>> {
    let by_size = len / min_segment.max(1);
    let count = (connections.clamp(1, MAX_CONNECTIONS) as u64).min(by_size);
    if count < 2 {
        return None;
    }
    let size = len / count;
    Some(
        (0..count)
            .map(|i| Segment {
                start: i * size,
                end: Some(if i + 1 == count {
                    len - 1
                } else {
                    (i + 1) * size - 1
                }),
                done: 0,
            })
            .collect(),
    )
}

/// How a segmented download ended.
pub(crate) enum Outcome {
    Finished(Finished),
    /// The download must start over; `response` is a whole-file `200` to
    /// start from, if one arrived.
    Restart {
        part: File,
        reason: RestartReason,
        response: Option<Response>,
        /// Handed back so a new download keeps its guard.
        cleanup: Option<EmptyDownloadCleanup>,
    },
}

/// What the engine hands over to download segments.
pub(crate) struct Job {
    pub client: Client,
    /// The browser's headers for this download (M5): kept in memory only.
    pub browser: Arc<crate::browser::FromBrowser>,
    pub connections: usize,
    /// No split leaves a segment smaller than this.
    pub min_segment: u64,
    pub save_interval: Duration,
    /// A segment whose connection fails this many times in a row, without
    /// writing in between, is given up.
    pub max_failures: u32,
    /// The caller's request to stop: every connection ends, and only then
    /// does [`run`] return. What is on disk is recorded unless a periodic
    /// save already did (#132): the state file counts only synced bytes.
    pub stop: Stop,
    /// The speed limits every connection goes through.
    pub limits: Limits,
    /// The counter of the download's site, if the user limited it
    /// ([`crate::sites`]): every connection after the first takes a slot.
    pub site: Option<Arc<tokio::sync::Semaphore>>,
    /// The URL range requests go to.
    pub url: Url,
    /// The `If-Range` value: a strong validator (see `if_range_value`).
    pub if_range: String,
    pub final_path: PathBuf,
    pub part: File,
    pub state: DownloadState,
    /// A response already streaming the segment that starts at the given
    /// byte, from that byte on: the first `200`, or the `206` that a resume
    /// checked.
    pub first: Option<(Response, u64)>,
    /// Set for a new download until its first byte is written.
    pub cleanup: Option<EmptyDownloadCleanup>,
}

/// Downloads every missing byte of `job.state.segments`.
pub(crate) async fn run(
    job: Job,
    on_event: &mut (dyn FnMut(Event) + Send),
) -> Result<Outcome, Error> {
    let Job {
        client,
        browser,
        connections,
        min_segment,
        save_interval,
        max_failures,
        stop: user_stop,
        limits,
        site,
        url,
        if_range,
        final_path,
        part,
        state,
        first,
        mut cleanup,
    } = job;
    let total = state
        .content_length
        .expect("segmented downloads have a size");
    let state_file = state_path(&final_path);
    let (events, mut messages) = mpsc::unbounded_channel();
    let (stop, stopped) = watch::channel(false);
    let connections = connections.clamp(1, MAX_CONNECTIONS);
    let mut board = Board::new(state, min_segment, connections, max_failures);
    if let Some((_, start)) = &first {
        let index = board
            .index_of(*start)
            .expect("the first response's segment");
        board.live[index].take(0, Instant::now());
        board.active = 1;
    }
    let ctx = Arc::new(Ctx {
        limits,
        site,
        client,
        browser,
        url,
        if_range,
        total,
        validator: board.state.validator.clone(),
        file: Arc::new(part),
        board: Mutex::new(board),
        new: Mutex::new(cleanup.as_ref().and_then(EmptyDownloadCleanup::watch)),
        events,
        stopped,
    });

    let from = ctx.board().state.bytes_done();
    on_event(Event::Started {
        file: final_path.clone(),
        state_file: state_file.clone(),
        total: Some(total),
        from,
    });

    let mut workers = JoinSet::new();
    let mut first = first;
    for id in 0..connections {
        let stream = match (id, first.take()) {
            (0, Some((response, start))) => Some(Stream {
                response,
                next: ctx.board().segment_write_position(start),
                last: total - 1,
                segment: start,
                // The first connection is the download's own slot.
                slot: None,
            }),
            (_, taken) => {
                first = taken;
                None
            }
        };
        workers.spawn(worker(ctx.clone(), id, stream));
    }

    let mut done = from;
    let mut restart = None;
    let mut stopped_by_user = false;
    // The disk failed: the workers are stopping, and no periodic save
    // starts (on a full disk it would fail too).
    let mut disk_failed = false;
    // A periodic save has succeeded: the state file counts only synced
    // bytes, so a stop needs no save of its own (#132).
    let mut saved = false;
    // A save every `save_interval`: the first one interval after the start
    // (an `interval` would tick at once), and the next one an interval
    // after the last (`Delay`). So at most about one interval of progress
    // is not in the state file (owner's decision on #132), plus how long a
    // slow disk takes to save.
    let mut tick =
        tokio::time::interval_at(tokio::time::Instant::now() + save_interval, save_interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            // A stop asked wins over a save that is also due (#194: with a
            // slow disk both were ready, and the save could be picked).
            biased;
            () = user_stop.requested(), if !stopped_by_user => {
                stopped_by_user = true;
                let _ = stop.send(true);
            }
            message = messages.recv() => match message {
                Some(Message::Wrote(bytes)) => {
                    done += bytes;
                    if let Some(cleanup) = cleanup.take() {
                        // The first byte is on disk: the files stay now.
                        cleanup.disarm();
                        *ctx.new.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    }
                    on_event(Event::Progress { done, total: Some(total) });
                }
                Some(Message::Split { segment, at }) => {
                    on_event(Event::Split { segment, at });
                }
                Some(Message::ServerBusy { status, connections, wait }) => {
                    on_event(Event::ServerBusy { status, connections, wait });
                }
                Some(Message::Restart { reason, response }) if restart.is_none() => {
                    restart = Some((reason, response));
                    let _ = stop.send(true);
                }
                Some(Message::Restart { .. }) | None => {}
                Some(Message::DiskFailed) => {
                    disk_failed = true;
                    let _ = stop.send(true);
                }
            },
            joined = workers.join_next() => match joined {
                Some(Ok(())) => {}
                Some(Err(err)) => {
                    // A worker panicked: stop the others and report it.
                    ctx.board().fail(None, format!("a connection failed: {err}"));
                    let _ = stop.send(true);
                }
                None => break,
            },
            // No periodic save once the download stops (asked by the user,
            // or for a restart): the workers are ending, and a save then
            // is the one #132 leaves out (#194). After the messages: an
            // event (a split) is said before the save that holds it (a
            // test stopping on a split's event relies on it).
            _ = tick.tick(), if !stopped_by_user && restart.is_none() && !disk_failed => {
                // A stop asked during a slow save reaches the workers at
                // once, not when the save is done (review of #208: they
                // wrote on for the whole save).
                let save = checkpoint(&ctx, &state_file, cleanup.as_ref());
                tokio::pin!(save);
                let result = loop {
                    tokio::select! {
                        biased;
                        () = user_stop.requested(), if !stopped_by_user => {
                            stopped_by_user = true;
                            let _ = stop.send(true);
                        }
                        result = &mut save => break result,
                    }
                };
                match result {
                    Ok(()) => saved = true,
                    // The state file saved last stays, and counts only
                    // bytes on disk: stop, resumable from it.
                    Err(err) => {
                        ctx.board().disk_failed(format!("{err}"));
                        disk_failed = true;
                        let _ = stop.send(true);
                    }
                }
            }
        }
    }
    // Messages sent just before the last worker ended.
    while let Ok(message) = messages.try_recv() {
        match message {
            Message::Wrote(_) => {
                if let Some(cleanup) = cleanup.take() {
                    cleanup.disarm();
                    *ctx.new.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
            }
            Message::Split { segment, at } => on_event(Event::Split { segment, at }),
            Message::ServerBusy {
                status,
                connections,
                wait,
            } => on_event(Event::ServerBusy {
                status,
                connections,
                wait,
            }),
            Message::Restart { reason, response } => {
                restart.get_or_insert((reason, response));
            }
            // The board has it (`Board::disk_failed`).
            Message::DiskFailed => {}
        }
    }

    if stopped_by_user {
        // Every worker has ended. A new download with nothing written is
        // removed when `cleanup` is dropped; nothing else holds it now.
        if cleanup.is_some() {
            return Err(Error::Stopped { state_file: None });
        }
        // Owner's decision on #132: after a periodic save, a stop does not
        // save again. On a slow disk (Windows, with an antivirus) a second
        // sync could take seconds; about one `save_interval` of progress is
        // downloaded again on resume, and nothing is lost or wrong.
        if !saved && !disk_failed {
            // A failed save leaves the state file saved last, which counts
            // only bytes on disk.
            let _ = checkpoint(&ctx, &state_file, None).await;
        }
        return Err(Error::Stopped {
            state_file: Some(state_file),
        });
    }

    if let Some((reason, response)) = restart {
        // Every worker has ended, so nothing else holds the file.
        let ctx = Arc::into_inner(ctx).expect("workers have ended");
        let part = Arc::into_inner(ctx.file).expect("no write is running");
        return Ok(Outcome::Restart {
            part,
            reason,
            response,
            cleanup,
        });
    }

    let (complete, failure, ranges_refused, disk_failed) = {
        let board = ctx.board();
        (
            board.complete(),
            board.failure.clone(),
            board.ranges_refused,
            board.disk_failed,
        )
    };
    if complete {
        let ctx = Arc::into_inner(ctx).expect("workers have ended");
        let state = ctx
            .board
            .into_inner()
            .unwrap_or_else(|e| e.into_inner())
            .state;
        let part = Arc::into_inner(ctx.file).expect("no write is running");
        return finish_download(part, state, final_path, on_event)
            .await
            .map(Outcome::Finished);
    }

    let reason = failure.unwrap_or_else(|| {
        if ranges_refused {
            "the server stopped accepting requests for parts of the file".into()
        } else {
            "the server stopped sending parts of the file".into()
        }
    });
    if cleanup.is_some() {
        // A new download with nothing written: its files are removed when
        // `cleanup` is dropped (see `EmptyDownloadCleanup`).
        return Err(Error::NoData(reason));
    }
    match checkpoint(&ctx, &state_file, None).await {
        Ok(()) => {}
        // On a full disk the save fails too. The state file saved last
        // counts only bytes that are on disk, so the download resumes from
        // there.
        Err(_) if disk_failed => {}
        Err(err) => return Err(err),
    }
    Err(Error::Interrupted { state_file, reason })
}

/// Syncs the data, then saves a state that counts only synced bytes.
async fn checkpoint(
    ctx: &Arc<Ctx>,
    state_file: &std::path::Path,
    cleanup: Option<&EmptyDownloadCleanup>,
) -> Result<(), Error> {
    let mut snapshot = ctx.board().state.clone();
    let file = ctx.file.clone();
    blocking(move || file.sync_data()).await?;
    let watch = cleanup.and_then(EmptyDownloadCleanup::watch);
    save_state(&mut snapshot, state_file, watch).await
}

/// What workers share.
struct Ctx {
    limits: Limits,
    /// See [`Job::site`].
    site: Option<Arc<tokio::sync::Semaphore>>,
    client: Client,
    browser: Arc<crate::browser::FromBrowser>,
    url: Url,
    if_range: String,
    total: u64,
    validator: Validator,
    file: Arc<File>,
    board: Mutex<Board>,
    /// While a new download has no data yet: writes keep it alive, so its
    /// files are removed only after the last write (see `NewDownload`).
    new: Mutex<Option<Arc<NewDownload>>>,
    events: mpsc::UnboundedSender<Message>,
    stopped: watch::Receiver<bool>,
}

impl Ctx {
    fn board(&self) -> MutexGuard<'_, Board> {
        self.board.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn should_stop(&self) -> bool {
        *self.stopped.borrow()
    }

    /// Resolves when the coordinator asks every worker to stop.
    async fn stop_requested(&self) {
        let mut stopped = self.stopped.clone();
        let _ = stopped.wait_for(|stop| *stop).await;
    }
}

enum Message {
    /// Bytes written to the file.
    Wrote(u64),
    /// The segment starting at `segment` was split; the new one starts at
    /// `at`.
    Split { segment: u64, at: u64 },
    /// The server answered `status` (`403` or `429`).
    ServerBusy {
        status: u16,
        connections: usize,
        wait: Option<Duration>,
    },
    Restart {
        reason: RestartReason,
        response: Option<Response>,
    },
    /// Writing the file failed (a full disk): every connection stops.
    DiskFailed,
}

/// The segments and who works on them.
struct Board {
    state: DownloadState,
    /// No split leaves a segment smaller than this.
    min_segment: u64,
    /// Connections allowed at once; drops when the server refuses more.
    limit: usize,
    /// Connections asking for or reading a response right now.
    active: usize,
    /// No new request before this moment (`Retry-After`).
    pause_until: Option<Instant>,
    /// One entry per segment of `state`, in the same order.
    live: Vec<Live>,
    /// The server answered a range request with the whole file.
    ranges_refused: bool,
    /// Why the first segment failed, or why the disk did.
    failure: Option<String>,
    /// Writing the file or saving the state failed (a full disk). Trying
    /// again would not help, so the download stops at once, resumable from
    /// the state file saved last once there is room again (owner's
    /// decision, 2026-10-28).
    disk_failed: bool,
    /// Failures in a row after which a segment is given up (at least 1).
    max_failures: u32,
}

#[derive(Debug, Default, Clone)]
struct Live {
    owner: Option<usize>,
    /// Bytes after `done` that the owner is writing right now.
    pending: u64,
    /// The segment was given up: no connection asks for it again.
    failed: bool,
    /// Failures in a row without writing anything in between.
    failures: u32,
    /// No connection asks for the segment again before this moment.
    retry_at: Option<Instant>,
    /// When the owner took the segment, and the bytes it wrote since: its
    /// speed.
    since: Option<Instant>,
    written: u64,
}

impl Live {
    fn take(&mut self, id: usize, now: Instant) {
        self.owner = Some(id);
        self.since = Some(now);
        self.written = 0;
    }
}

/// What a connection with no free segment can do.
#[derive(Debug, PartialEq, Eq)]
enum Steal {
    /// It took the new segment `(start, end)`, split from the segment that
    /// starts at `from`.
    Split { from: u64, start: u64, end: u64 },
    /// Nothing to split yet; a segment may become worth splitting soon.
    Wait,
    /// Nothing will ever be worth splitting: the connection can end.
    Nothing,
}

impl Board {
    fn new(state: DownloadState, min_segment: u64, connections: usize, max_failures: u32) -> Self {
        let live = vec![Live::default(); state.segments.len()];
        Board {
            state,
            min_segment: min_segment.max(1),
            limit: connections.max(1),
            active: 0,
            pause_until: None,
            live,
            ranges_refused: false,
            failure: None,
            disk_failed: false,
            max_failures: max_failures.max(1),
        }
    }

    fn index_of(&self, start: u64) -> Option<usize> {
        self.state
            .segments
            .binary_search_by_key(&start, |s| s.start)
            .ok()
    }

    fn end_of(&self, index: usize) -> u64 {
        self.state.segments[index]
            .end
            .expect("segments have an end")
    }

    /// The next byte to write in the segment starting at `start`.
    fn segment_write_position(&self, start: u64) -> u64 {
        let index = self.index_of(start).expect("segment exists");
        let segment = &self.state.segments[index];
        segment.start + segment.done + self.live[index].pending
    }

    fn is_complete(&self, index: usize) -> bool {
        let segment = &self.state.segments[index];
        segment.start + segment.done > self.end_of(index)
    }

    fn complete(&self) -> bool {
        (0..self.live.len()).all(|i| self.is_complete(i))
    }

    /// Takes the first segment nobody works on that still needs bytes and
    /// may be asked for now.
    fn take_free(&mut self, id: usize, now: Instant) -> Option<(u64, u64, u64)> {
        let index = (0..self.live.len()).find(|&i| {
            let live = &self.live[i];
            live.owner.is_none()
                && !live.failed
                && live.retry_at.is_none_or(|at| at <= now)
                && !self.is_complete(i)
        })?;
        self.live[index].take(id, now);
        let segment = &self.state.segments[index];
        Some((
            segment.start,
            segment.start + segment.done,
            self.end_of(index),
        ))
    }

    /// Lets go of the segment starting at `start` if `id` owns it.
    fn release(&mut self, id: usize, start: u64) {
        if let Some(index) = self.index_of(start)
            && self.live[index].owner == Some(id)
        {
            self.live[index].owner = None;
        }
    }

    /// The earliest moment a segment waiting for a retry may be asked for.
    fn soonest_retry(&self) -> Option<Instant> {
        (0..self.live.len())
            .filter(|&i| {
                let live = &self.live[i];
                live.owner.is_none() && !live.failed && !self.is_complete(i)
            })
            .filter_map(|i| self.live[i].retry_at)
            .min()
    }

    /// The connection on the segment starting at `start` failed for
    /// `reason`: the segment waits `wait` (or a growing wait) before another
    /// try, or is given up after too many failures in a row.
    fn retry_later(
        &mut self,
        id: usize,
        start: u64,
        reason: String,
        now: Instant,
        wait: Option<Duration>,
    ) {
        let Some(index) = self.index_of(start) else {
            return;
        };
        let live = &mut self.live[index];
        if live.owner.is_some_and(|owner| owner != id) {
            // Another connection took it over before anything was written.
            return;
        }
        live.owner = None;
        live.failures = live.failures.saturating_add(1);
        if live.failures >= self.max_failures {
            let failures = live.failures;
            self.fail(
                Some(start),
                format!("{reason} (gave up after {failures} tries)"),
            );
        } else {
            let wait = wait.unwrap_or_else(|| retry::backoff(live.failures));
            live.retry_at = now.checked_add(wait);
        }
    }

    /// The server refused a connection while others worked: allow only as
    /// many as it accepted. Returns the new limit.
    fn fewer_connections(&mut self) -> usize {
        self.limit = self.limit.min(self.active.saturating_sub(1)).max(1);
        self.limit
    }

    /// The disk failed (`reason`): the reason the user needs, even after
    /// another one.
    fn disk_failed(&mut self, reason: String) {
        if !self.disk_failed {
            self.disk_failed = true;
            self.failure = Some(reason);
        }
    }

    /// Marks the segment starting at `start` failed (if any) and remembers
    /// the first reason.
    fn fail(&mut self, start: Option<u64>, reason: String) {
        if let Some(index) = start.and_then(|start| self.index_of(start)) {
            self.live[index].failed = true;
            self.live[index].owner = None;
        }
        self.failure.get_or_insert(reason);
    }

    /// Reserves up to `wanted` bytes at `at` for `id`, in the segment that
    /// starts at `start`. Returns how many may be written: 0 if `id` no
    /// longer owns the segment or it is full.
    fn reserve(&mut self, id: usize, start: u64, at: u64, wanted: u64) -> u64 {
        let Some(index) = self.index_of(start) else {
            return 0;
        };
        if self.live[index].owner != Some(id) {
            return 0;
        }
        let segment = &self.state.segments[index];
        let next = segment.start + segment.done + self.live[index].pending;
        if next != at {
            return 0;
        }
        let room = (self.end_of(index) + 1).saturating_sub(next);
        let bytes = wanted.min(room);
        self.live[index].pending += bytes;
        bytes
    }

    /// Counts `bytes` reserved by [`Board::reserve`] as written.
    fn wrote(&mut self, start: u64, bytes: u64) {
        let index = self.index_of(start).expect("a reserved segment stays");
        let live = &mut self.live[index];
        live.pending -= bytes;
        live.written += bytes;
        live.failures = 0;
        live.retry_at = None;
        self.state.segments[index].done += bytes;
    }

    /// Bytes of segment `index` that nobody has written or reserved yet.
    fn remaining(&self, index: usize) -> u64 {
        let segment = &self.state.segments[index];
        let next = segment.start + segment.done + self.live[index].pending;
        (self.end_of(index) + 1).saturating_sub(next)
    }

    /// For a connection with no free segment: splits the segment with the
    /// longest time left (remaining bytes ÷ the speed of its connection)
    /// and gives `id` its second half. A segment is split only if both
    /// halves keep at least `min_segment` bytes; a connection that has just
    /// started is not judged until its speed is known, unless it is
    /// stalled.
    fn split_slowest(&mut self, id: usize, now: Instant) -> Steal {
        if self.ranges_refused {
            return Steal::Nothing;
        }
        let mut best: Option<(usize, f64)> = None;
        let mut may_wait = false;
        for index in 0..self.live.len() {
            let live = &self.live[index];
            let remaining = self.remaining(index);
            if live.owner.is_none() || live.owner == Some(id) || remaining / 2 < self.min_segment {
                continue;
            }
            let elapsed = live.since.map_or(Duration::ZERO, |since| now - since);
            let seconds_left = if live.written == 0 {
                if elapsed < STALLED_AFTER {
                    may_wait = true;
                    continue;
                }
                f64::INFINITY
            } else {
                if elapsed < SPEED_AFTER {
                    may_wait = true;
                    continue;
                }
                let speed = live.written as f64 / elapsed.as_secs_f64();
                remaining as f64 / speed
            };
            if best.is_none_or(|(_, longest)| seconds_left > longest) {
                best = Some((index, seconds_left));
            }
        }
        let Some((index, _)) = best else {
            return if may_wait {
                Steal::Wait
            } else {
                Steal::Nothing
            };
        };
        let segment = self.state.segments[index].clone();
        let end = self.end_of(index);
        let remaining = self.remaining(index);
        // The new segment starts halfway through what is left.
        let start = end + 1 - remaining / 2;
        self.state.segments[index].end = Some(start - 1);
        self.state.segments.insert(
            index + 1,
            Segment {
                start,
                end: Some(end),
                done: 0,
            },
        );
        let mut live = Live::default();
        live.take(id, now);
        self.live.insert(index + 1, live);
        Steal::Split {
            from: segment.start,
            start,
            end,
        }
    }

    /// Gives back a reservation that was not written.
    fn unreserve(&mut self, start: u64, bytes: u64) {
        if let Some(index) = self.index_of(start) {
            self.live[index].pending -= bytes;
        }
    }

    /// When `id` has filled the segment starting at `start`: takes the next
    /// segment if it begins right after and nothing of it is written or
    /// reserved, so the same response goes on. Returns its start.
    fn continue_into_next(&mut self, id: usize, start: u64, last: u64) -> Option<u64> {
        let index = self.index_of(start)?;
        if self.live[index].owner != Some(id) || !self.is_complete(index) {
            return None;
        }
        let next_start = self.end_of(index) + 1;
        if next_start > last {
            return None;
        }
        let next = index + 1;
        let segment = self.state.segments.get(next)?;
        let live = &self.live[next];
        if segment.done != 0 || live.pending != 0 {
            return None;
        }
        // Whoever owned it has not written anything; its connection lets go
        // as soon as it notices.
        self.live[next] = Live::default();
        self.live[next].take(id, Instant::now());
        self.release(id, start);
        Some(next_start)
    }
}

/// A response being read into the file.
struct Stream {
    response: Response,
    /// File offset of the next byte the response delivers.
    next: u64,
    /// File offset of the last byte the response delivers.
    last: u64,
    /// Start of the segment this stream writes.
    segment: u64,
    /// This connection's slot of a limited site, if it needed one: given
    /// back when the stream ends.
    slot: Option<tokio::sync::OwnedSemaphorePermit>,
}

/// One connection: reads its stream into the file, then asks for more work
/// until there is none.
async fn worker(ctx: Arc<Ctx>, id: usize, mut stream: Option<Stream>) {
    loop {
        if ctx.should_stop() {
            return;
        }
        let current = match stream.take() {
            Some(stream) => stream,
            None => match find_work(&ctx, id).await {
                Work::Stream(stream) => stream,
                Work::Again => continue,
                Work::Done => return,
            },
        };
        let result = pump(&ctx, id, current).await;
        let mut board = ctx.board();
        board.active -= 1;
        if let Err((segment, reason)) = result {
            board.retry_later(id, segment, reason, Instant::now(), None);
        }
    }
}

enum Work {
    Stream(Stream),
    /// Look again (after a split that came to nothing, or a short wait).
    Again,
    /// Nothing left for this connection.
    Done,
}

/// A free segment, or else the second half of the slowest one. On
/// [`Work::Stream`] the connection holds one of the allowed slots
/// (`Board::active`) until its stream ends.
async fn find_work(ctx: &Arc<Ctx>, id: usize) -> Work {
    let now = Instant::now();
    let (found, slot) = {
        let mut board = ctx.board();
        if board.ranges_refused || board.active >= board.limit {
            // Nothing to ask for, or the server allows fewer connections.
            return Work::Done;
        }
        // On a site the user limited, a connection besides another one of
        // this download needs a slot of its own (`crate::sites`). Without
        // one it waits and looks again, as another download may give one
        // back (review of #166: it used to end, so the download never grew
        // again). Once this download's other connections have all ended, it
        // needs none: so it never waits for nothing.
        let slot = match (&ctx.site, board.active >= 1) {
            (Some(site), true) => site.clone().try_acquire_owned().map(Some).ok(),
            _ => Some(None),
        };
        let paused = board.pause_until.filter(|&until| until > now);
        let found = match (&slot, paused) {
            (None, _) => Err(now + SITE_LOOK_AGAIN),
            (_, Some(until)) => Err(until),
            (Some(_), None) => match board.take_free(id, now) {
                Some(free) => Ok(free),
                None => match board.soonest_retry() {
                    Some(at) => Err(at),
                    None => match board.split_slowest(id, now) {
                        Steal::Split { from, start, end } => {
                            let _ = ctx.events.send(Message::Split {
                                segment: from,
                                at: start,
                            });
                            Ok((start, start, end))
                        }
                        Steal::Wait => Err(now + LOOK_AGAIN),
                        Steal::Nothing => return Work::Done,
                    },
                },
            },
        };
        if found.is_ok() {
            board.active += 1;
        }
        (found, slot.flatten())
    };
    let (start, from, to) = match found {
        Ok(work) => work,
        Err(until) => {
            // No slot of the site is held while waiting (review of #166).
            drop(slot);
            return wait_then_again(ctx, until).await;
        }
    };
    match request_range(ctx, id, start, from, to).await {
        Some(mut stream) => {
            stream.slot = slot;
            Work::Stream(stream)
        }
        None => {
            // The segment failed, waits or was given back: another one may
            // still need this connection.
            ctx.board().active -= 1;
            Work::Again
        }
    }
}

/// Waits until `until` (or a stop), then looks for work again.
async fn wait_then_again(ctx: &Arc<Ctx>, until: Instant) -> Work {
    tokio::select! {
        _ = tokio::time::sleep_until(until.into()) => {}
        _ = ctx.stop_requested() => {}
    }
    Work::Again
}

/// Asks the server for bytes `from` to `to` of the segment starting at
/// `start`, which `id` owns. `None` when the answer cannot be used.
async fn request_range(
    ctx: &Arc<Ctx>,
    id: usize,
    start: u64,
    from: u64,
    to: u64,
) -> Option<Stream> {
    let request = ctx
        .browser
        .apply(&ctx.url, ctx.client.get(ctx.url.clone()))
        .header(header::RANGE, format!("bytes={from}-{to}"))
        .header(header::IF_RANGE, &ctx.if_range)
        .send();
    let response = tokio::select! {
        // A stop wins over an answer that is also ready (#194).
        biased;
        _ = ctx.stop_requested() => {
            ctx.board().release(id, start);
            return None;
        }
        response = request => response,
    };
    let response = match response {
        Ok(response) => response,
        Err(err) => {
            ctx.board()
                .retry_later(id, start, http_error(err), Instant::now(), None);
            return None;
        }
    };
    match response.status() {
        StatusCode::PARTIAL_CONTENT => {
            let range = response
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_content_range);
            let exact =
                range.is_some_and(|r| r.start == from && r.end == to && r.total == Some(ctx.total));
            if changed_from(&ctx.validator, Some(ctx.total), &response) {
                restart(ctx, RestartReason::FileChanged, None);
                None
            } else if exact {
                Some(Stream {
                    response,
                    next: from,
                    last: to,
                    segment: start,
                    slot: None,
                })
            } else {
                ctx.board().fail(
                    Some(start),
                    "the server sent a different part than requested".into(),
                );
                None
            }
        }
        StatusCode::OK if changed_from(&ctx.validator, Some(ctx.total), &response) => {
            restart(ctx, RestartReason::FileChanged, Some(response));
            None
        }
        StatusCode::OK => {
            // The server ignores ranges. The connection reading from the
            // start (if any) goes on into this segment.
            let mut board = ctx.board();
            board.ranges_refused = true;
            board.release(id, start);
            None
        }
        StatusCode::TOO_MANY_REQUESTS => {
            too_many_requests(ctx, id, start, &response);
            None
        }
        StatusCode::FORBIDDEN => {
            let mut board = ctx.board();
            if board.active > 1 {
                // Others work: the server limits connections. The segment
                // goes back to them.
                let connections = board.fewer_connections();
                board.release(id, start);
                drop(board);
                let _ = ctx.events.send(Message::ServerBusy {
                    status: 403,
                    connections,
                    wait: None,
                });
            } else {
                board.fail(Some(start), "the server answered 403 Forbidden".into());
            }
            None
        }
        status if status.is_server_error() || status == StatusCode::REQUEST_TIMEOUT => {
            ctx.board().retry_later(
                id,
                start,
                format!("the server answered {status}"),
                Instant::now(),
                None,
            );
            None
        }
        status => {
            ctx.board()
                .fail(Some(start), format!("the server answered {status}"));
            None
        }
    }
}

/// `429 Too Many Requests`: no request to the server before its
/// `Retry-After` (or a growing wait without one), and fewer connections if
/// others are working. A wait longer than [`retry::LONGEST_RETRY_AFTER`]
/// gives the segment up, so the download stops and can be resumed later.
fn too_many_requests(ctx: &Ctx, id: usize, start: u64, response: &Response) {
    let now = Instant::now();
    let asked = response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| retry::retry_after(v, SystemTime::now()));
    let mut board = ctx.board();
    if let Some(wait) = asked.filter(|&wait| wait > retry::LONGEST_RETRY_AFTER) {
        board.fail(
            Some(start),
            format!(
                "the server asks to wait {} s before more requests; resume the download later",
                wait.as_secs()
            ),
        );
        return;
    }
    let connections = if board.active > 1 {
        board.fewer_connections()
    } else {
        board.limit
    };
    let failures = board
        .index_of(start)
        .map_or(0, |index| board.live[index].failures);
    let wait = asked.unwrap_or_else(|| retry::backoff(failures.saturating_add(1)));
    let until = now.checked_add(wait);
    board.pause_until = board.pause_until.max(until);
    board.retry_later(
        id,
        start,
        "the server answered 429 Too Many Requests".into(),
        now,
        Some(wait),
    );
    drop(board);
    let _ = ctx.events.send(Message::ServerBusy {
        status: 429,
        connections,
        wait: Some(wait),
    });
}

fn restart(ctx: &Ctx, reason: RestartReason, response: Option<Response>) {
    let _ = ctx.events.send(Message::Restart { reason, response });
}

/// Writes `stream` into its segment, and on into the next ones while they
/// are untouched. Ends without error when the segment (or the response)
/// ends, or another connection took the segment; `Err` with the reason when
/// the connection failed.
async fn pump(ctx: &Arc<Ctx>, id: usize, mut stream: Stream) -> Result<(), (u64, String)> {
    read_into_file(ctx, id, &mut stream)
        .await
        .map_err(|reason| (stream.segment, reason))
}

async fn read_into_file(ctx: &Arc<Ctx>, id: usize, stream: &mut Stream) -> Result<(), String> {
    let mut batch: Vec<u8> = Vec::with_capacity(WRITE_BATCH);
    let mut batch_at = stream.next;
    let mut last_write = Instant::now();
    loop {
        let chunk = tokio::select! {
            // A stop wins over data that is also ready: from a fast server
            // there is nearly always more, and a connection that took it
            // went on downloading after the stop (#130, #136, #194).
            biased;
            _ = ctx.stop_requested() => {
                let _ = write_batch(ctx, id, stream, &mut batch, &mut batch_at).await;
                ctx.board().release(id, stream.segment);
                return Ok(());
            }
            chunk = stream.response.chunk() => chunk,
        };
        let chunk = match chunk {
            Ok(Some(chunk)) => chunk,
            Ok(None) => {
                write_batch(ctx, id, stream, &mut batch, &mut batch_at).await?;
                ctx.board().release(id, stream.segment);
                return if stream.next > stream.last {
                    Ok(())
                } else {
                    Err(format!(
                        "the connection closed at byte {} of a part that ends at {}",
                        stream.next, stream.last
                    ))
                };
            }
            Err(err) => {
                let _ = write_batch(ctx, id, stream, &mut batch, &mut batch_at).await;
                return Err(http_error(err));
            }
        };
        // Never past what was asked for.
        let room = (stream.last + 1).saturating_sub(stream.next);
        if chunk.len() as u64 > room {
            return Err("the server sent more data than requested".into());
        }
        batch.extend_from_slice(&chunk);
        stream.next += chunk.len() as u64;
        // Past the end of our segment (it may have been split): write now,
        // to learn at once whether this stream goes on into the next one,
        // rather than read on into bytes it may have to drop (M3: with a
        // speed limit, every byte read counts).
        let past_end = {
            let board = ctx.board();
            board
                .index_of(stream.segment)
                .is_some_and(|index| stream.next > board.end_of(index) + 1)
        };
        // The speed limits, for each piece as it arrives: wait before
        // reading more (review of #32: taking only once a batch of 256 KiB
        // was written let every connection's first batch through at full
        // speed). Bytes later dropped, not written, are given back.
        tokio::select! {
            biased;
            _ = ctx.stop_requested() => {
                let _ = write_batch(ctx, id, stream, &mut batch, &mut batch_at).await;
                ctx.board().release(id, stream.segment);
                return Ok(());
            }
            () = ctx.limits.take(chunk.len() as u64) => {}
        }
        // Under a speed limit every piece is written at once: a batch that
        // a split leaves behind is dropped, and near the end of a download
        // nothing reads on to use the time it cost (CI measured 0.89 of
        // the limit with 256 KiB batches). Limited, the writes are few.
        let limited = ctx.limits.active();
        if (past_end
            || limited
            || batch.len() >= WRITE_BATCH
            || last_write.elapsed() >= WRITE_AFTER)
            && !write_batch(ctx, id, stream, &mut batch, &mut batch_at).await?
        {
            // The segment is full and the next one is not ours to take,
            // or another connection took this one.
            ctx.board().release(id, stream.segment);
            return Ok(());
        }
        if past_end || batch.is_empty() {
            last_write = Instant::now();
        }
    }
}

/// Writes the batch into the stream's segment, moving on into the next
/// segment when this one fills up. Returns `false` when the stream cannot
/// go on (the rest of the batch is dropped).
async fn write_batch(
    ctx: &Arc<Ctx>,
    id: usize,
    stream: &mut Stream,
    batch: &mut Vec<u8>,
    batch_at: &mut u64,
) -> Result<bool, String> {
    let mut offset = 0usize;
    while offset < batch.len() {
        let at = *batch_at + offset as u64;
        let wanted = (batch.len() - offset) as u64;
        let granted = ctx.board().reserve(id, stream.segment, at, wanted);
        if granted == 0 {
            let next = ctx
                .board()
                .continue_into_next(id, stream.segment, stream.last);
            match next {
                Some(next) if next == at => {
                    stream.segment = next;
                    continue;
                }
                _ => {
                    // Read but never written: not counted by the limits.
                    ctx.limits.give_back((batch.len() - offset) as u64);
                    batch.clear();
                    *batch_at = stream.next;
                    return Ok(false);
                }
            }
        }
        let piece = batch[offset..offset + granted as usize].to_vec();
        let file = ctx.file.clone();
        let keep_alive = ctx.new.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let written = blocking(move || {
            let result = zajil_platform::fs::write_at(&file, &piece, at);
            // A cancelled new download removes its files only after this
            // write is over, and after this handle is closed.
            drop(file);
            drop(keep_alive);
            result
        })
        .await;
        {
            let mut board = ctx.board();
            match written {
                Ok(()) => board.wrote(stream.segment, granted),
                Err(err) => {
                    board.unreserve(stream.segment, granted);
                    let reason = format!("could not write the file: {err}");
                    board.disk_failed(reason.clone());
                    drop(board);
                    let _ = ctx.events.send(Message::DiskFailed);
                    return Err(reason);
                }
            }
        }
        let _ = ctx.events.send(Message::Wrote(granted));
        offset += granted as usize;
    }
    batch.clear();
    *batch_at = stream.next;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_equal_segments_that_cover_the_file() {
        let segments = plan(10 << 20, 8, 1 << 20).unwrap();
        assert_eq!(segments.len(), 8);
        assert_eq!(segments[0].start, 0);
        assert_eq!(segments.last().unwrap().end, Some((10 << 20) - 1));
        for pair in segments.windows(2) {
            assert_eq!(pair[0].end.unwrap() + 1, pair[1].start);
        }
    }

    #[test]
    fn plans_fewer_segments_for_small_files_and_none_below_two() {
        assert_eq!(plan(3 << 20, 8, 1 << 20).unwrap().len(), 3);
        assert!(plan((2 << 20) - 1, 8, 1 << 20).is_none());
        assert!(plan(100 << 20, 1, 1 << 20).is_none());
        assert!(plan(0, 8, 1 << 20).is_none());
    }

    /// A board of `segments` of 1000 bytes each, all owned, the owner of
    /// segment `i` having worked `ages[i]` and written `written[i]`.
    fn board(ages: &[Duration], written: &[u64], now: Instant) -> Board {
        let count = ages.len() as u64;
        let mut state = DownloadState::new(
            "http://h/f".into(),
            "http://h/f".into(),
            Validator::default(),
            Some(count * 1000),
            true,
        );
        state.segments = (0..count)
            .map(|i| Segment {
                start: i * 1000,
                end: Some(i * 1000 + 999),
                done: written[i as usize],
            })
            .collect();
        let mut board = Board::new(state, 100, 8, retry::MAX_FAILURES);
        for (i, live) in board.live.iter_mut().enumerate() {
            live.take(i, now - ages[i]);
            live.written = written[i];
        }
        board
    }

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn splits_the_segment_with_the_longest_time_left_in_half() {
        let now = Instant::now();
        // Same age; segment 1 has written least, so it is the slowest.
        let mut board = board(&[SECOND; 3], &[500, 100, 400], now);
        let steal = board.split_slowest(9, now);
        // 900 bytes remain of segment 1 (100..=999 of it); the new segment
        // takes the last 450.
        assert_eq!(
            steal,
            Steal::Split {
                from: 1000,
                start: 1550,
                end: 1999
            }
        );
        let segments = &board.state.segments;
        assert_eq!(segments[1].end, Some(1549));
        assert_eq!((segments[2].start, segments[2].end), (1550, Some(1999)));
        assert_eq!(segments[2].done, 0);
        assert_eq!(board.live[2].owner, Some(9));
        assert_eq!(board.live.len(), segments.len());
    }

    #[test]
    fn a_stalled_connection_is_split_first() {
        let now = Instant::now();
        let mut board = board(&[2 * SECOND, 2 * SECOND], &[10, 0], now);
        assert!(matches!(
            board.split_slowest(9, now),
            Steal::Split { from: 1000, .. }
        ));
    }

    #[test]
    fn waits_while_connections_are_too_new_to_judge() {
        let now = Instant::now();
        let young = Duration::from_millis(100);
        let mut board = board(&[young, young], &[10, 0], now);
        assert_eq!(board.split_slowest(9, now), Steal::Wait);
    }

    #[test]
    fn never_leaves_a_half_below_the_minimum() {
        let now = Instant::now();
        // 150 bytes left in each: halves of 75 < 100.
        let mut board = board(&[SECOND; 2], &[850, 850], now);
        assert_eq!(board.split_slowest(9, now), Steal::Nothing);
        // A free segment or one owned by the asker is never split.
        let mut board = super::tests::board(&[SECOND; 2], &[0, 0], now);
        board.live[0].owner = None;
        board.live[1].owner = Some(9);
        assert_eq!(board.split_slowest(9, now), Steal::Nothing);
    }

    #[test]
    fn nothing_is_split_once_the_server_refuses_ranges() {
        let now = Instant::now();
        let mut board = board(&[SECOND; 2], &[0, 0], now);
        board.ranges_refused = true;
        assert_eq!(board.split_slowest(9, now), Steal::Nothing);
    }

    #[test]
    fn never_plans_more_than_32_connections_and_survives_huge_sizes() {
        assert_eq!(plan(u64::MAX, 1000, 1).unwrap().len(), MAX_CONNECTIONS);
        let segments = plan(u64::MAX, 32, 0).unwrap();
        assert_eq!(segments.last().unwrap().end, Some(u64::MAX - 1));
    }
}
