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

use std::fs::File;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use reqwest::header;
use reqwest::{Client, Response, StatusCode, Url};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

use crate::engine::{
    EmptyDownloadCleanup, Error, Event, Finished, NewDownload, RestartReason, blocking,
    changed_from, error_chain, finish_download, parse_content_range, save_state,
};
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
    pub connections: usize,
    pub save_interval: Duration,
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
pub(crate) async fn run(job: Job, on_event: &mut dyn FnMut(Event)) -> Result<Outcome, Error> {
    let Job {
        client,
        connections,
        save_interval,
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
    let mut board = Board::new(state);
    if let Some((_, start)) = &first {
        let index = board
            .index_of(*start)
            .expect("the first response's segment");
        board.live[index].owner = Some(0);
    }
    let ctx = Arc::new(Ctx {
        client,
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
    for id in 0..connections.clamp(1, MAX_CONNECTIONS) {
        let stream = match (id, first.take()) {
            (0, Some((response, start))) => Some(Stream {
                response,
                next: ctx.board().segment_write_position(start),
                last: total - 1,
                segment: start,
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
    let mut last_save = Instant::now();
    let mut tick = tokio::time::interval(save_interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
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
                Some(Message::Restart { reason, response }) if restart.is_none() => {
                    restart = Some((reason, response));
                    let _ = stop.send(true);
                }
                Some(Message::Restart { .. }) | None => {}
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
            _ = tick.tick() => {
                if last_save.elapsed() >= save_interval {
                    checkpoint(&ctx, &state_file, cleanup.as_ref()).await?;
                    last_save = Instant::now();
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
            Message::Restart { reason, response } => {
                restart.get_or_insert((reason, response));
            }
        }
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

    let (complete, failure, ranges_refused) = {
        let board = ctx.board();
        (
            board.complete(),
            board.failure.clone(),
            board.ranges_refused,
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
    checkpoint(&ctx, &state_file, None).await?;
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
    client: Client,
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
    Restart {
        reason: RestartReason,
        response: Option<Response>,
    },
}

/// The segments and who works on them.
struct Board {
    state: DownloadState,
    /// One entry per segment of `state`, in the same order.
    live: Vec<Live>,
    /// The server answered a range request with the whole file.
    ranges_refused: bool,
    /// Why the first segment failed.
    failure: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct Live {
    owner: Option<usize>,
    /// Bytes after `done` that the owner is writing right now.
    pending: u64,
    /// A connection failed on this segment; no other one retries it (M2
    /// retries arrive with backoff in a later change).
    failed: bool,
}

impl Board {
    fn new(state: DownloadState) -> Self {
        let live = vec![Live::default(); state.segments.len()];
        Board {
            state,
            live,
            ranges_refused: false,
            failure: None,
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

    /// Takes the first segment nobody works on that still needs bytes.
    fn take_free(&mut self, id: usize) -> Option<(u64, u64, u64)> {
        let index = (0..self.live.len()).find(|&i| {
            let live = &self.live[i];
            live.owner.is_none() && !live.failed && !self.is_complete(i)
        })?;
        self.live[index].owner = Some(id);
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
        self.live[index].pending -= bytes;
        self.state.segments[index].done += bytes;
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
        self.live[next] = Live {
            owner: Some(id),
            ..Live::default()
        };
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
            None => match request_free_segment(&ctx, id).await {
                Some(stream) => stream,
                None => return,
            },
        };
        let segment = current.segment;
        if let Err(reason) = pump(&ctx, id, current).await {
            ctx.board().fail(Some(segment), reason);
            return;
        }
    }
}

/// Asks the server for the next free segment. `None` when there is nothing
/// left to ask for, or the answer cannot be used.
async fn request_free_segment(ctx: &Arc<Ctx>, id: usize) -> Option<Stream> {
    let (start, from, to) = {
        let mut board = ctx.board();
        if board.ranges_refused {
            return None;
        }
        board.take_free(id)?
    };
    let request = ctx
        .client
        .get(ctx.url.clone())
        .header(header::RANGE, format!("bytes={from}-{to}"))
        .header(header::IF_RANGE, &ctx.if_range)
        .send();
    let response = tokio::select! {
        response = request => response,
        _ = ctx.stop_requested() => {
            ctx.board().release(id, start);
            return None;
        }
    };
    let response = match response {
        Ok(response) => response,
        Err(err) => {
            ctx.board().fail(Some(start), error_chain(&err));
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
        status => {
            ctx.board()
                .fail(Some(start), format!("the server answered {status}"));
            None
        }
    }
}

fn restart(ctx: &Ctx, reason: RestartReason, response: Option<Response>) {
    let _ = ctx.events.send(Message::Restart { reason, response });
}

/// Writes `stream` into its segment, and on into the next ones while they
/// are untouched. Ends without error when the segment (or the response)
/// ends, or another connection took the segment; `Err` with the reason when
/// the connection failed.
async fn pump(ctx: &Arc<Ctx>, id: usize, mut stream: Stream) -> Result<(), String> {
    let mut batch: Vec<u8> = Vec::with_capacity(WRITE_BATCH);
    let mut batch_at = stream.next;
    let mut last_write = Instant::now();
    loop {
        let chunk = tokio::select! {
            chunk = stream.response.chunk() => chunk,
            _ = ctx.stop_requested() => {
                let _ = write_batch(ctx, id, &mut stream, &mut batch, &mut batch_at).await;
                ctx.board().release(id, stream.segment);
                return Ok(());
            }
        };
        let chunk = match chunk {
            Ok(Some(chunk)) => chunk,
            Ok(None) => {
                write_batch(ctx, id, &mut stream, &mut batch, &mut batch_at).await?;
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
                let _ = write_batch(ctx, id, &mut stream, &mut batch, &mut batch_at).await;
                return Err(error_chain(&err));
            }
        };
        // Never past what was asked for.
        let room = (stream.last + 1).saturating_sub(stream.next);
        if chunk.len() as u64 > room {
            return Err("the server sent more data than requested".into());
        }
        batch.extend_from_slice(&chunk);
        stream.next += chunk.len() as u64;
        if batch.len() >= WRITE_BATCH || last_write.elapsed() >= WRITE_AFTER {
            if !write_batch(ctx, id, &mut stream, &mut batch, &mut batch_at).await? {
                // The segment is full and the next one is not ours to take,
                // or another connection took this one.
                ctx.board().release(id, stream.segment);
                return Ok(());
            }
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
        let mut board = ctx.board();
        match written {
            Ok(()) => board.wrote(stream.segment, granted),
            Err(err) => {
                board.unreserve(stream.segment, granted);
                return Err(format!("could not write the file: {err}"));
            }
        }
        drop(board);
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

    #[test]
    fn never_plans_more_than_32_connections_and_survives_huge_sizes() {
        assert_eq!(plan(u64::MAX, 1000, 1).unwrap().len(), MAX_CONNECTIONS);
        let segments = plan(u64::MAX, 32, 0).unwrap();
        assert_eq!(segments.last().unwrap().end, Some(u64::MAX - 1));
    }
}
