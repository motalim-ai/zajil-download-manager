//! Video downloads in the app (M6, #149, ADR-0009): the tools (yt-dlp and
//! FFmpeg) installed once the user agrees, a page's video asked about
//! (its formats, or a playlist's items), and its download, with progress
//! told to the page. The engine is `zajil_core::video` and
//! `zajil_core::tools`, on the same folders as `zajil video`
//! (`<data>/tools`, `<data>/run`).
//!
//! A video from the browser extension comes with the cookies of its page
//! only (ADR-0005): they stay here, in memory, by link, and never reach
//! the page; the probe and the download take them from here.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};
use ts_rs::TS;
use zajil_core::Stop;

use crate::view::{DetailsView, DownloadView, StatusView};
use zajil_core::tools::{Tool, Toolbox};
use zajil_core::video::{
    Choice, Download, Format, Probed, Runs, ToolProblem, VideoError, VideoEvent, VideoRunner,
};

const TOOLS: [Tool; 4] = Tool::ALL;

/// What the app keeps for video downloads.
#[derive(Default)]
pub struct VideoState {
    /// How many run at once, from the settings (0 until read: the
    /// default).
    at_once: AtomicUsize,
    /// A page's cookies, by link, as the extension sent them.
    cookies: Mutex<HashMap<String, String>>,
    /// A page's video's own link, by page, as the extension saw it: asked
    /// when yt-dlp cannot read the page.
    media: Mutex<HashMap<String, String>>,
    /// The link a page was read by when it was its video's own: the
    /// download goes from it.
    read_as: Mutex<HashMap<String, String>>,
    /// Pages from a private window: their downloads are never written to
    /// `videos.json`, and leave the list when the last private window
    /// closes.
    private: Mutex<HashSet<String>>,
    /// The formats the last probe of each link found (a download names
    /// them).
    formats: Mutex<HashMap<String, Vec<Format>>>,
    /// The link «Video quality» opens with, from the extension.
    pending: Mutex<Option<String>>,
    installing: Mutex<Option<Stop>>,
    /// One install, update or removal of the tools at a time (review of
    /// #278: the update at start and the user's install raced).
    tools_lock: tokio::sync::Mutex<()>,
    probing: Mutex<Option<Stop>>,
    downloads: Mutex<HashMap<u64, Stop>>,
    /// Downloads whose formats «Video quality» just read: their page is
    /// not asked again when they start.
    fresh: Mutex<HashSet<u64>>,
    next: AtomicU64,
    /// The video downloads of the list (#149): kept in `videos.json`.
    rows: Mutex<Vec<Row>>,
    /// When each running one last told the list (at most four times a
    /// second, as the queues' downloads).
    told: Mutex<HashMap<u64, Instant>>,
    /// The videos read ahead, while the pointer was on them.
    ahead: Mutex<Aheads>,
    /// The download a format chosen in the browser started, while «Video
    /// quality» asks where it goes (as IDM's «Download file info», owner,
    /// 2026-10-29): it runs meanwhile.
    started: Mutex<Option<u64>>,
    /// Downloads cancelled while they ran: once stopped, they leave the
    /// list with what they wrote.
    discard: Mutex<HashSet<u64>>,
}

/// How long a video read ahead is kept (owner, 2026-10-28): in memory
/// only, never written anywhere.
const AHEAD_FOR: Duration = Duration::from_secs(10 * 60);
/// At most this many are read ahead at once; another one meanwhile is not
/// (yt-dlp solving YouTube's JavaScript takes seconds of a core each).
const AHEAD_RUNNING: usize = 2;
/// At most this many are kept; the oldest goes first.
const AHEAD_KEPT: usize = 20;

/// A read ahead: none while it runs, then what it found (none if it
/// failed: the click asks again).
type AheadRead = tokio::sync::watch::Receiver<Option<Option<zajil_core::video::Sent>>>;
type AheadDone = tokio::sync::watch::Sender<Option<Option<zajil_core::video::Sent>>>;

/// The videos read ahead (owner, 2026-10-28, as IDM): when the extension's
/// button shows over a video, its page is read in the background once, so
/// its qualities are there when the button is pressed. Never from a
/// private window (the extension sends none, and the host refuses them).
#[derive(Default)]
struct Aheads(HashMap<String, Ahead>);

struct Ahead {
    at: Instant,
    cookies: bool,
    read: AheadRead,
}

impl Aheads {
    fn forget_old(&mut self, now: Instant) {
        self.0
            .retain(|_, a| now.saturating_duration_since(a.at) < AHEAD_FOR);
    }

    /// Notes a read of `page` and returns where to put what it found;
    /// none when it was read in the last ten minutes, or is being read,
    /// or [`AHEAD_RUNNING`] are running.
    fn begin(&mut self, page: &str, cookies: bool, now: Instant) -> Option<AheadDone> {
        self.forget_old(now);
        let running = self.0.values().filter(|a| a.read.borrow().is_none());
        if self.0.contains_key(page) || running.count() >= AHEAD_RUNNING {
            return None;
        }
        if self.0.len() >= AHEAD_KEPT {
            let oldest = self.0.iter().min_by_key(|(_, a)| a.at);
            if let Some(oldest) = oldest.map(|(page, _)| page.clone()) {
                self.0.remove(&oldest);
            }
        }
        let (done, read) = tokio::sync::watch::channel(None);
        let ahead = Ahead {
            at: now,
            cookies,
            read,
        };
        self.0.insert(page.to_owned(), ahead);
        Some(done)
    }

    /// The read of `page` the button's press can take: one of the last
    /// ten minutes, with cookies if the press has some and without if not
    /// (signed in or not, yt-dlp finds other formats). What it found must
    /// still fit the press ([`ahead_fits`]).
    fn find(&mut self, page: &str, cookies: bool, now: Instant) -> Option<AheadRead> {
        self.forget_old(now);
        self.0
            .get(page)
            .filter(|a| a.cookies == cookies)
            .map(|a| a.read.clone())
    }
}

/// Whether what a read ahead found fits a press on `page` with the video's
/// own link `media`: the page itself was read, or the video's link the
/// press has too (the one seen at the press may be another, YouTube's
/// above all: only the page counts then).
fn ahead_fits(sent: &zajil_core::video::Sent, page: &str, media: Option<&str>) -> bool {
    sent.url == page || media == Some(sent.url.as_str())
}

/// How many video downloads run at once: the settings' number, 1 to 8
/// (owner's request, 2026-10-29; it was always 2); the others wait in the
/// list (a playlist's items, many at a time).
fn at_once(state: &VideoState) -> usize {
    match state.at_once.load(Ordering::Relaxed) {
        0 => zajil_core::settings::DEFAULT_VIDEOS_AT_ONCE as usize,
        n => n,
    }
}

/// The settings' number of videos at once, read at the start.
pub fn keep_at_once(state: &VideoState, n: u32) {
    state.at_once.store(n as usize, Ordering::Relaxed);
}

/// The number changed in the settings: the waiting ones that may start
/// now start.
pub fn set_at_once<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, n: u32) {
    keep_at_once(&handle.state::<VideoState>(), n);
    pump(handle);
}

/// Video downloads' ids start here, far above the queues' (JavaScript's
/// numbers stay exact up to 2^53), so one list holds both.
pub const FIRST_ID: u64 = 1 << 52;

pub fn is_video(id: u64) -> bool {
    id >= FIRST_ID
}

/// Splits ids into the videos' and the queues'.
pub fn split(ids: Vec<u64>) -> (Vec<u64>, Vec<u64>) {
    ids.into_iter().partition(|&id| is_video(id))
}

const ROWS_FILE: &str = "videos.json";

/// One video download of the list. A stopped one starts over (yt-dlp's
/// run folder is swept), so «resume» asks the page again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Row {
    id: u64,
    /// The page's link (never shown in a log).
    url: String,
    /// The name without extension, as asked; the file's once finished.
    name: String,
    folder: String,
    choice: ChoiceView,
    status: StatusView,
    size: Option<u64>,
    done: u64,
    #[serde(skip)]
    speed: u64,
    error: Option<String>,
    file: Option<String>,
    /// From a private window: never written to `videos.json`.
    #[serde(skip)]
    private: bool,
    /// Where the user wants it once finished, if not where it downloads:
    /// changed in «Video quality» while it was already downloading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    move_to: Option<MoveTo>,
    /// The quality chosen in the browser's list, as it said it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    /// When it joined the list (RFC 3339), for the list's newest first;
    /// none in a list saved before (owner's request, 2026-10-30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    added: Option<String>,
}

impl Row {
    /// Whether it lands where `to` says anyway: the same folder, and the
    /// same name (or none given).
    fn lands_at(&self, to: &MoveTo) -> bool {
        self.folder == to.folder && (to.name.is_empty() || to.name == self.name)
    }
}

/// A finished video's folder and name, as the user changed them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MoveTo {
    folder: String,
    name: String,
}

fn rows_file(app: &crate::App) -> PathBuf {
    app.data_dir.join(ROWS_FILE)
}

/// At start: the list's video downloads. One that was running when Zajil
/// stopped is paused (its run was swept).
pub fn load_rows(app: &crate::App, state: &VideoState) {
    let rows = read_rows(&rows_file(app));
    let last = rows.iter().map(|r| r.id - FIRST_ID).max().unwrap_or(0);
    state.next.store(last, Ordering::SeqCst);
    *state.rows.lock().unwrap() = rows;
}

fn read_rows(path: &std::path::Path) -> Vec<Row> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(mut rows) = serde_json::from_str::<Vec<Row>>(&text) else {
        eprintln!("zajil: {ROWS_FILE} could not be read; the video downloads start empty");
        return Vec::new();
    };
    // Never an id of the queues' (a file changed by hand).
    rows.retain(|r| is_video(r.id));
    for row in &mut rows {
        // What came stays shown: Zajil's own way continues from it.
        if matches!(row.status, StatusView::Active | StatusView::Waiting) {
            row.status = StatusView::Paused;
        }
    }
    rows
}

/// Writes the list's video downloads whole (a new file, then renamed),
/// but those of a private window.
fn save_rows(app: &crate::App, state: &VideoState) {
    let rows = kept(&state.rows.lock().unwrap());
    let path = rows_file(app);
    let saved = serde_json::to_vec_pretty(&rows)
        .map_err(std::io::Error::other)
        .and_then(|bytes| {
            crate::saving::write_whole(&path, &path.with_extension("json.new"), &bytes)
        });
    if let Err(e) = saved {
        eprintln!("zajil: {ROWS_FILE} could not be saved: {e}");
    }
}

/// The rows `videos.json` keeps: not those of a private window.
fn kept(rows: &[Row]) -> Vec<Row> {
    rows.iter().filter(|r| !r.private).cloned().collect()
}

/// The last private window closed: its video downloads leave the list (a
/// running one stops; a finished file stays), as the queues' do.
pub fn discard_private<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    let state = handle.state::<VideoState>();
    let ids: Vec<u64> = state
        .rows
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.private)
        .map(|r| r.id)
        .collect();
    state.private.lock().unwrap().clear();
    if !ids.is_empty() {
        remove(handle, &ids);
    }
}

fn with_row<T>(state: &VideoState, id: u64, f: impl FnOnce(&mut Row) -> T) -> Option<T> {
    state
        .rows
        .lock()
        .unwrap()
        .iter_mut()
        .find(|r| r.id == id)
        .map(f)
}

/// The list's rows for the video downloads, newest last.
pub fn views(
    state: &VideoState,
    categories: &zajil_core::category::CategoriesFile,
) -> Vec<DownloadView> {
    state
        .rows
        .lock()
        .unwrap()
        .iter()
        .map(|row| view_of(row, categories))
        .collect()
}

fn view_of(row: &Row, categories: &zajil_core::category::CategoriesFile) -> DownloadView {
    let host = crate::view::host_of(&row.url);
    let name = row
        .file
        .as_deref()
        .and_then(|f| std::path::Path::new(f).file_name())
        .map_or_else(|| row.name.clone(), |n| n.to_string_lossy().into_owned());
    // Its category is the videos' (or the sounds' for «audio only»), as
    // a name with an extension would give.
    let probe = match (&row.file, &row.choice) {
        (Some(_), _) => name.clone(),
        (None, ChoiceView::AudioOnly) => format!("{name}.m4a"),
        (None, _) => format!("{name}.mp4"),
    };
    DownloadView {
        id: row.id,
        category: categories
            .classify(&probe, host.as_deref())
            .map(|c| c.id.clone()),
        name,
        host,
        size: row.size,
        done: row.done,
        speed: if row.status == StatusView::Active {
            row.speed as f64
        } else {
            0.0
        },
        status: row.status,
        queue: String::new(),
        folder: row.folder.clone(),
        parts: Vec::new(),
        segments: 0,
        error: row.error.clone(),
        waits_for_browser: false,
        waits_for_site: false,
        description: None,
        last_try: None,
        added: row.added.clone(),
        needs_ok: false,
        video: true,
    }
}

/// The progress window's view of a video download.
pub fn details(
    state: &VideoState,
    categories: &zajil_core::category::CategoriesFile,
    id: u64,
) -> Option<DetailsView> {
    let row = with_row(state, id, |row| row.clone())?;
    // Zajil's own way (YouTube) continues from its parts; yt-dlp's starts
    // over. Its parts are there once it began, its file once it finished.
    let resumable = row.status == StatusView::Done
        || !zajil_core::video::part_files(std::path::Path::new(&row.folder), &row.name).is_empty();
    // Not known before its first byte (owner's report, 2026-10-29: «not
    // possible» at the start of a download that could).
    let known = resumable || row.done > 0;
    Some(DetailsView {
        download: view_of(&row, categories),
        url: row.url.clone(),
        segments: Vec::new(),
        resumable: known.then_some(resumable),
        limit: None,
    })
}

/// A finished video's file, and its folder.
pub fn file_of(state: &VideoState, id: u64) -> Option<(Option<PathBuf>, PathBuf)> {
    with_row(state, id, |row| {
        (
            row.file.as_ref().map(PathBuf::from).filter(|f| f.is_file()),
            PathBuf::from(&row.folder),
        )
    })
}

/// A video download's page link («Copy link»).
pub fn url_of(state: &VideoState, id: u64) -> Option<String> {
    with_row(state, id, |row| row.url.clone())
}

/// «Pause»: a running video stops (it starts over when resumed); a
/// waiting one stays out of the turn.
pub fn pause(state: &VideoState, ids: &[u64]) {
    for &id in ids {
        with_row(state, id, |row| {
            if row.status == StatusView::Waiting {
                row.status = StatusView::Paused;
            }
        });
    }
    let downloads = state.downloads.lock().unwrap();
    for id in ids {
        if let Some(stop) = downloads.get(id) {
            stop.request();
        }
    }
}

/// «Resume»: a stopped or failed video starts again, from asking its page.
pub fn resume<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, ids: &[u64]) {
    let state = handle.state::<VideoState>();
    for &id in ids {
        with_row(&state, id, |row| {
            if matches!(row.status, StatusView::Paused | StatusView::Failed) {
                row.status = StatusView::Waiting;
                row.error = None;
            }
        });
    }
    save_rows(&handle.state::<crate::App>(), &state);
    let _ = handle.emit("downloads-changed", ());
    pump(handle);
}

/// The waiting video downloads that may start now, first come first: as
/// many as leave at most `most` running.
fn to_start(rows: &[Row], running: &HashSet<u64>, most: usize) -> Vec<u64> {
    let free = most.saturating_sub(running.len());
    rows.iter()
        .filter(|r| r.status == StatusView::Waiting && !running.contains(&r.id))
        .map(|r| r.id)
        .take(free)
        .collect()
}

/// Starts the waiting video downloads whose turn it is.
fn pump<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    let state = handle.state::<VideoState>();
    let running = state.downloads.lock().unwrap().keys().copied().collect();
    let ids = to_start(&state.rows.lock().unwrap(), &running, at_once(&state));
    for id in ids {
        // A page «Video quality» asked just now is not asked again.
        let ask = !state.fresh.lock().unwrap().remove(&id);
        start(handle.clone(), id, ask);
    }
}

/// Takes video downloads off the list; a running one stops first. A
/// finished file stays; a stopped run leaves nothing.
pub fn remove<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, ids: &[u64]) {
    let state = handle.state::<VideoState>();
    pause(&state, ids);
    state.rows.lock().unwrap().retain(|r| !ids.contains(&r.id));
    save_rows(&handle.state::<crate::App>(), &state);
    let _ = handle.emit("downloads-changed", ());
}

/// «Delete» with the unfinished files: an unfinished video goes as a
/// cancelled one does, with its parts (Zajil's own way leaves them in its
/// folder; yt-dlp's run is swept); a finished one leaves the list and
/// keeps its file.
pub fn remove_with_files<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, ids: &[u64]) {
    let state = handle.state::<VideoState>();
    let (unfinished, finished) = by_finished(&state.rows.lock().unwrap(), ids);
    remove(handle, &finished);
    for id in unfinished {
        discard(handle, id);
    }
}

/// `ids` split into those not finished and those finished; an id not in
/// `rows` is among the finished (only taken off the list).
fn by_finished(rows: &[Row], ids: &[u64]) -> (Vec<u64>, Vec<u64>) {
    ids.iter().copied().partition(|&id| {
        rows.iter()
            .any(|r| r.id == id && r.status != StatusView::Done)
    })
}

fn folders(app: &crate::App) -> (PathBuf, PathBuf) {
    (app.data_dir.join("tools"), app.data_dir.join("run"))
}

/// A runner, with the runtime for YouTube's JavaScript as the CLI finds it:
/// Deno or Node on `PATH`, else our QuickJS-NG once it is installed (#155).
async fn runner(app: &crate::App) -> VideoRunner {
    let (tools, runs) = folders(app);
    let toolbox = Toolbox::new(tools);
    let has_qjs = toolbox.is_installed(Tool::QuickJs);
    let js = tauri::async_runtime::spawn_blocking(move || {
        let jsbox = std::env::current_exe()
            .ok()
            .and_then(|exe| zajil_core::video::jsbox_beside(&exe))
            .filter(|_| has_qjs);
        let path = std::env::var_os("PATH");
        zajil_core::video::find_js_runtime(path.as_deref(), jsbox, false)
    })
    .await
    .ok()
    .flatten();
    VideoRunner::new(toolbox, Runs::new(runs), app.engine.clone()).with_js(js)
}

/// At start: what an unfinished run or install left is swept away, with no
/// word unless it fails (#149).
pub fn clean_leftovers(app: &crate::App) {
    let (tools, runs) = folders(app);
    if let Err(e) = Toolbox::new(tools).remove_leftovers() {
        eprintln!("zajil: the video tools' leftovers: {e}");
    }
    if let Err(e) = Runs::new(runs).clean_leftovers() {
        eprintln!("zajil: the video runs' leftovers: {e}");
    }
}

/// «Video quality»'s window, and its capability: events, its own title,
/// and the folder picker.
pub const LABEL: &str = "video";
pub const SIZE: (f64, f64) = (640.0, 520.0);
pub const CAPABILITY: &str = include_str!("../runtime-capabilities/video.json");

/// A video from the extension: its cookies and its own link kept here,
/// and «Video quality» opened for its page; or, with a format chosen in
/// the button's list, its download started at once in the videos' folder,
/// named after the page's title once it is read, with its progress window.
pub fn receive<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    url: String,
    media: Option<String>,
    cookies: Option<String>,
    private: bool,
    chosen: Option<Chosen>,
) {
    let state = handle.state::<VideoState>();
    if let Some(cookies) = cookies {
        state.cookies.lock().unwrap().insert(url.clone(), cookies);
    }
    if let Some(media) = media {
        state.media.lock().unwrap().insert(url.clone(), media);
    }
    if private {
        state.private.lock().unwrap().insert(url.clone());
    }
    if let Some(Chosen {
        choice,
        label,
        size,
    }) = chosen
    {
        let app = handle.state::<crate::App>();
        // The same video and quality, stopped: it goes on from what came
        // (with the cookies the browser just sent), not again from nothing.
        let stopped = state
            .rows
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|r| {
                r.url == url
                    && matches!(r.status, StatusView::Paused | StatusView::Failed)
                    && same_itags(&r.choice, &choice)
            })
            .map(|r| (r.id, r.name.clone()));
        if let Some((id, name)) = stopped {
            resume(handle, &[id]);
            let show = handle
                .state::<crate::closing::Closing>()
                .prefs()
                .add
                .progress_window;
            if let Err(err) = crate::addprefs::show_started(handle, show, id, &name) {
                eprintln!("zajil: the progress window of video {id}: {err}");
            }
            return;
        }
        let id = FIRST_ID + state.next.fetch_add(1, Ordering::SeqCst) + 1;
        state.rows.lock().unwrap().push(Row {
            id,
            url,
            // The page's title, once read (`start`).
            name: String::new(),
            folder: crate::download_dir(&app.settings, &app.system_download_dir)
                .display()
                .to_string(),
            choice,
            status: StatusView::Waiting,
            size,
            done: 0,
            speed: 0,
            error: None,
            file: None,
            private,
            move_to: None,
            label,
            added: Some(zajil_core::state::now_rfc3339()),
        });
        save_rows(&app, &state);
        let _ = handle.emit("downloads-changed", ());
        pump(handle);
        // An earlier one still asked about goes on where it is.
        *state.started.lock().unwrap() = Some(id);
        let _ = handle.emit_to(LABEL, "video-started", id);
        let _ = open_window(handle);
        return;
    }
    *state.pending.lock().unwrap() = Some(url);
    let _ = open_window(handle);
}

/// A quality chosen in the browser's list: its formats (YouTube's itags),
/// and how the list said it, with its size.
pub struct Chosen {
    pub choice: ChoiceView,
    pub label: Option<String>,
    pub size: Option<u64>,
}

/// Whether two choices are the same formats, by YouTube's itags (yt-dlp
/// names a format `140-0` where the list says `140`).
fn same_itags(a: &ChoiceView, b: &ChoiceView) -> bool {
    let itag = |id: &str| id.split('-').next().unwrap_or_default().to_owned();
    match (a, b) {
        (ChoiceView::Format { id: x }, ChoiceView::Format { id: y }) => itag(x) == itag(y),
        (
            ChoiceView::Merge {
                video: v1,
                audio: a1,
            },
            ChoiceView::Merge {
                video: v2,
                audio: a2,
            },
        ) => itag(v1) == itag(v2) && itag(a1) == itag(a2),
        _ => false,
    }
}

/// A download from the browser's list, for «Video quality» while it asks
/// where it goes.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct StartedView {
    #[ts(type = "number")]
    pub id: u64,
    /// The page's title once read, else empty.
    pub name: String,
    pub folder: String,
    pub status: StatusView,
    /// The quality, as the browser's list said it.
    pub label: Option<String>,
    #[ts(type = "number | null")]
    pub size: Option<u64>,
    #[ts(type = "number")]
    pub done: u64,
    pub error: Option<String>,
}

/// The download «Video quality» asks about, if any.
#[tauri::command]
pub fn video_started(state: State<'_, VideoState>) -> Option<StartedView> {
    let id = (*state.started.lock().unwrap())?;
    with_row(&state, id, |row| StartedView {
        id,
        name: row.name.clone(),
        folder: row.folder.clone(),
        status: row.status,
        label: row.label.clone(),
        size: row.size,
        done: row.done,
        error: row.error.clone(),
    })
}

/// «Start download»: it goes on, to `folder` as `name` once finished (at
/// once if it already is). Its progress window takes over.
#[tauri::command]
pub async fn video_keep<R: tauri::Runtime>(
    handle: tauri::AppHandle<R>,
    id: u64,
    folder: String,
    name: String,
) -> Result<(), String> {
    let state = handle.state::<VideoState>();
    state.started.lock().unwrap().take_if(|s| *s == id);
    let name = zajil_core::sanitize::sanitize_file_name(name.trim());
    let wanted = MoveTo { folder, name };
    let done_file = with_row(&state, id, |row| match (&row.file, row.status) {
        (Some(file), StatusView::Done) => {
            (!row.lands_at(&wanted)).then(|| (PathBuf::from(file), row.name.clone()))
        }
        // Moved there once finished, if it is not where it lands then.
        _ => {
            row.move_to = Some(wanted.clone());
            None
        }
    })
    .flatten();
    if let Some((file, title)) = done_file {
        let to = wanted.clone();
        let moved = tauri::async_runtime::spawn_blocking(move || {
            let name = if to.name.is_empty() { title } else { to.name };
            zajil_core::video::move_video(&file, std::path::Path::new(&to.folder), &name)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
        with_row(&state, id, |row| {
            row.file = Some(moved.display().to_string());
            row.folder = wanted.folder.clone();
        });
    }
    save_rows(&handle.state::<crate::App>(), &state);
    let _ = handle.emit("downloads-changed", ());
    Ok(())
}

/// «Download later»: it stops, and waits paused in the list, to go to
/// `folder` as `name` once it finishes; «Resume» goes on from what came.
#[tauri::command]
pub async fn video_later<R: tauri::Runtime>(
    handle: tauri::AppHandle<R>,
    id: u64,
    folder: String,
    name: String,
) -> Result<(), String> {
    video_keep(handle.clone(), id, folder, name).await?;
    let state = handle.state::<VideoState>();
    pause(&state, &[id]);
    with_row(&state, id, |row| {
        if row.status == StatusView::Waiting {
            row.status = StatusView::Paused;
        }
    });
    Ok(())
}

/// The download «Video quality» asks about, hidden from the list until
/// the user says «Start download» (as IDM: nothing shows it runs before).
pub fn asked(state: &VideoState) -> Option<u64> {
    *state.started.lock().unwrap()
}

/// «Cancel» while it asks where the download goes: it stops, and leaves
/// the list with what it wrote.
#[tauri::command]
pub fn video_discard<R: tauri::Runtime>(handle: tauri::AppHandle<R>, id: u64) {
    handle
        .state::<VideoState>()
        .started
        .lock()
        .unwrap()
        .take_if(|s| *s == id);
    discard(&handle, id);
}

/// Download `id` stops if it runs and leaves the list; its parts, and its
/// file if it finished, are removed (it was never kept).
fn discard<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, id: u64) {
    let state = handle.state::<VideoState>();
    let running = state.downloads.lock().unwrap().get(&id).cloned();
    if let Some(stop) = running {
        // `start` removes it once stopped.
        state.discard.lock().unwrap().insert(id);
        stop.request();
        return;
    }
    let Some(row) = with_row(&state, id, |row| row.clone()) else {
        return;
    };
    state.rows.lock().unwrap().retain(|r| r.id != id);
    save_rows(&handle.state::<crate::App>(), &state);
    let _ = handle.emit("downloads-changed", ());
    pump(handle);
    tauri::async_runtime::spawn_blocking(move || remove_leftovers(&row));
}

/// What a cancelled download wrote: its parts, and its file if finished.
fn remove_leftovers(row: &Row) {
    let folder = std::path::Path::new(&row.folder);
    for part in zajil_core::video::part_files(folder, &row.name) {
        let _ = zajil_platform::fs::remove_file_retrying(&part);
    }
    if let Some(file) = &row.file {
        let _ = zajil_platform::fs::remove_file_retrying(std::path::Path::new(file));
    }
}

/// `choice` if this read of the page still offers its formats, else the
/// engine's best: YouTube numbers an HLS format anew at each read (`96-2`,
/// then `96-1`), so a stopped download chosen so could never go on (owner's
/// report, 2026-10-29: «the chosen format is not one the probe found»).
/// Nothing read (no formats): as it was.
fn still_offered(choice: &ChoiceView, formats: &[Format]) -> ChoiceView {
    let there = |id: &String| formats.iter().any(|f| &f.id == id);
    let offered = match choice {
        ChoiceView::Format { id } => there(id),
        ChoiceView::Merge { video, audio } => there(video) && there(audio),
        ChoiceView::AudioOnly | ChoiceView::Best => true,
    };
    if offered || formats.is_empty() {
        choice.clone()
    } else {
        ChoiceView::Best
    }
}

/// The format of `formats` that YouTube's itag `wanted` names: yt-dlp
/// names a format by its itag, with a suffix for another language's sound
/// or a "stable volume" one (`140-drc`), the plain one first.
fn by_itag(formats: &[Format], wanted: &str) -> Option<String> {
    formats
        .iter()
        .find(|f| f.id == wanted)
        .or_else(|| {
            formats.iter().find(|f| {
                f.id.split_once('-')
                    .is_some_and(|(itag, rest)| itag == wanted && !rest.contains("drc"))
            })
        })
        .map(|f| f.id.clone())
}

/// A choice made in the browser's list, in the names of yt-dlp's formats;
/// the engine's best when the probe did not find what was chosen.
fn from_list(choice: &ChoiceView, formats: &[Format]) -> ChoiceView {
    let found = match choice {
        ChoiceView::Format { id } => by_itag(formats, id).map(|id| ChoiceView::Format { id }),
        ChoiceView::Merge { video, audio } => by_itag(formats, video)
            .zip(by_itag(formats, audio))
            .map(|(video, audio)| ChoiceView::Merge { video, audio }),
        other => Some(other.clone()),
    };
    found.unwrap_or(ChoiceView::Best)
}

/// Opens «Video quality» (or brings it to the front), titled in the
/// page's language by the page itself. It stays in front of the other
/// windows until it is done (the owner's notes on 1.0: it went behind the
/// browser and nothing brought it back), and the main window shows that
/// it waits («video-window»). Closed, it gives up the check of the page or
/// the tools' download still running, as «Download file info» does.
pub fn open_window<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) -> tauri::Result<()> {
    let new = handle.get_webview_window(LABEL).is_none();
    crate::windows::open(handle, LABEL, "Zajil", SIZE)?;
    let Some(window) = handle.get_webview_window(LABEL) else {
        return Ok(());
    };
    window.set_always_on_top(true)?;
    if new {
        let _ = handle.emit_to("main", "video-window", true);
        let app = handle.clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::Destroyed = event {
                give_up(&app.state::<VideoState>());
                // Closed while it asked where the download goes: cancelled.
                let started = app.state::<VideoState>().started.lock().unwrap().take();
                if let Some(id) = started {
                    discard(&app, id);
                }
                let _ = app.emit_to("main", "video-window", false);
            }
        });
    }
    Ok(())
}

/// The window closed: its check of a page and its tools' download stop.
/// The downloads it started go on in the list.
fn give_up(state: &VideoState) {
    for running in [&state.probing, &state.installing] {
        if let Some(stop) = running.lock().unwrap().take() {
            stop.request();
        }
    }
}

/// «Video quality» is open: the main window says a video waits there.
#[tauri::command]
pub fn video_window_open<R: tauri::Runtime>(handle: tauri::AppHandle<R>) -> bool {
    handle.get_webview_window(LABEL).is_some()
}

/// The main window's «Show»: «Video quality» comes to the front.
#[tauri::command]
pub async fn show_video_window<R: tauri::Runtime>(
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    if handle.get_webview_window(LABEL).is_none() {
        return Ok(());
    }
    open_window(&handle).map_err(|e| e.to_string())
}

/// «Download a video…»: «Video quality» for a link the user gives there.
#[tauri::command]
pub async fn open_video<R: tauri::Runtime>(
    state: State<'_, VideoState>,
    url: Option<String>,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    if crate::in_scene(&handle) {
        return Ok(());
    }
    *state.pending.lock().unwrap() = url;
    open_window(&handle).map_err(|e| e.to_string())
}

/// Where a video goes at first: the downloads folder.
#[tauri::command]
pub fn video_folder(app: State<'_, crate::App>) -> String {
    crate::download_dir(&app.settings, &app.system_download_dir)
        .display()
        .to_string()
}

/// The link the extension sent, once (the page opens «Video quality»).
#[tauri::command]
pub fn video_pending(state: State<'_, VideoState>) -> Option<String> {
    state.pending.lock().unwrap().take()
}

/// One tool, for the settings and for the question before installing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ToolView {
    /// `yt-dlp`, `ffmpeg` or `ffprobe`.
    pub tool: String,
    pub installed: bool,
    /// The version Zajil installs.
    pub version: Option<String>,
    /// Where it comes from: the host only.
    pub host: Option<String>,
    /// Where its source is, for «About» (ADR-0009): the release of the
    /// pinned version, or the project's page.
    pub source: String,
}

/// The page of the release `pin` comes from, or the project's own.
fn source_of(tool: Tool, pin: Option<&zajil_core::tools::Pinned>) -> String {
    if let Some(page) = pin.and_then(|p| {
        let (release, _file) = p.url.rsplit_once('/')?;
        release
            .contains("/releases/download/")
            .then(|| release.replacen("/releases/download/", "/releases/tag/", 1))
    }) {
        return page;
    }
    match tool {
        Tool::YtDlp => "https://github.com/yt-dlp/yt-dlp",
        Tool::QuickJs => "https://github.com/quickjs-ng/quickjs",
        // Our own build, with its exact source, on Zajil's releases.
        Tool::Ffmpeg | Tool::Ffprobe => {
            "https://github.com/motalim-ai/zajil-download-manager/releases"
        }
    }
    .into()
}

fn name(tool: Tool) -> &'static str {
    match tool {
        Tool::YtDlp => "yt-dlp",
        Tool::Ffmpeg => "ffmpeg",
        Tool::Ffprobe => "ffprobe",
        Tool::QuickJs => "qjs",
    }
}

/// Off the main thread: checking yt-dlp reads and hashes every file it
/// unpacked (review of #278), and the windows would wait meanwhile.
#[tauri::command]
pub async fn video_tools_status(app: State<'_, crate::App>) -> Result<Vec<ToolView>, String> {
    let toolbox = Toolbox::new(folders(&app).0);
    tauri::async_runtime::spawn_blocking(move || tools_status(&toolbox))
        .await
        .map_err(|e| e.to_string())
}

/// Why `tool` is not ready, checked again a moment later before it is
/// asked for: a tool that was there was asked for again (owner's reports,
/// 2026-10-29), and a check that failed once is said in the log. Blocking.
fn checked_twice(toolbox: &Toolbox, tool: Tool) -> Option<zajil_core::tools::ToolError> {
    let first = toolbox.why_not_installed(tool)?;
    // Never there: no second look.
    if matches!(first, zajil_core::tools::ToolError::Missing(_))
        && !toolbox
            .path(tool)
            .is_ok_and(|p| p.parent().is_some_and(std::path::Path::exists))
    {
        return Some(first);
    }
    eprintln!("zajil: {} failed its check once: {first}", name(tool));
    std::thread::sleep(std::time::Duration::from_millis(300));
    toolbox.why_not_installed(tool)
}

fn tools_status(toolbox: &Toolbox) -> Vec<ToolView> {
    let target = zajil_platform::target::current();
    TOOLS
        .iter()
        .map(|&tool| {
            let pin = target.and_then(|t| zajil_core::tools::pinned(tool, t));
            let why = checked_twice(toolbox, tool);
            if let Some(why) = &why {
                eprintln!("zajil: {} is not ready: {why}", name(tool));
            }
            ToolView {
                tool: name(tool).into(),
                installed: why.is_none(),
                version: pin.as_ref().map(|p| p.version.clone()),
                host: pin
                    .as_ref()
                    .and_then(|p| zajil_core::Url::parse(&p.url).ok())
                    .and_then(|u| u.host_str().map(str::to_owned)),
                source: source_of(tool, pin.as_ref()),
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
struct ToolProgress {
    tool: &'static str,
    done: u64,
    total: Option<u64>,
}

/// Installs the tools that are missing, in order, once the user agreed;
/// `video-tools-progress` says how far. Nothing half-downloaded stays.
/// Then yt-dlp is started once, offline, still under «preparing the tools»
/// ([`VideoRunner::warm_up`]), so its slow first start is not the first
/// video's.
#[tauri::command]
pub async fn video_tools_install<R: tauri::Runtime>(
    app: State<'_, crate::App>,
    state: State<'_, VideoState>,
    handle: tauri::AppHandle<R>,
) -> Result<(), VideoErrorView> {
    let stop = Stop::new();
    *state.installing.lock().unwrap() = Some(stop.clone());
    // After an update at start that is under way: then what it brought
    // is there, and only what is still missing comes.
    let _one_at_a_time = state.tools_lock.lock().await;
    let toolbox = Toolbox::new(folders(&app).0);
    let mut installed = false;
    for tool in TOOLS {
        if toolbox.is_installed(tool) {
            continue;
        }
        installed = true;
        let events = handle.clone();
        toolbox
            .install(&app.engine, tool, &stop, &mut |event| {
                if let zajil_core::Event::Progress { done, total } = event {
                    let _ = events.emit(
                        "video-tools-progress",
                        ToolProgress {
                            tool: name(tool),
                            done,
                            total,
                        },
                    );
                }
            })
            .await
            .map_err(|e| VideoErrorView {
                kind: "tool".into(),
                message: e.to_string(),
            })?;
    }
    if installed {
        warm_up(&app, &stop).await?;
    }
    state.installing.lock().unwrap().take();
    Ok(())
}

/// [`VideoRunner::warm_up`] after an install: a yt-dlp that did not get as
/// far as loading what it needs, or that could not be checked or started
/// at all, fails the preparing of the tools in the user's words
/// (`not_ready`, owner's decision on #279), with why under «Details»; a
/// stop is said too.
async fn warm_up(app: &crate::App, stop: &Stop) -> Result<(), VideoErrorView> {
    match runner(app).await.warm_up(stop).await {
        Ok(()) => Ok(()),
        Err(e @ (VideoError::Stopped | VideoError::NotReady(_))) => Err(e.into()),
        Err(e) => Err(VideoError::NotReady(e.to_string()).into()),
    }
}

/// At start: a tool this user had before in another version or build
/// (yt-dlp's one-file build, now that its zip is pinned) is brought up to
/// date in the background, without asking again: they agreed to it then
/// (owner's decision, 2026-10-27). Then yt-dlp is warmed up. Said only in
/// the log.
pub fn update_tools<R: tauri::Runtime>(handle: tauri::AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let app = handle.state::<crate::App>();
        let state = handle.state::<VideoState>();
        let _one_at_a_time = state.tools_lock.lock().await;
        let tools = folders(&app).0;
        let earlier = tauri::async_runtime::spawn_blocking(move || {
            let toolbox = Toolbox::new(tools);
            TOOLS
                .into_iter()
                .filter(|&tool| toolbox.has_an_earlier_one(tool))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        if earlier.is_empty() {
            return;
        }
        let stop = Stop::new();
        let toolbox = Toolbox::new(folders(&app).0);
        for tool in earlier {
            match toolbox.install(&app.engine, tool, &stop, &mut |_| {}).await {
                Ok(path) => eprintln!("zajil: {tool} brought up to date: {}", path.display()),
                Err(e) => {
                    eprintln!("zajil: {tool} could not be brought up to date: {e}");
                    return;
                }
            }
        }
        let _ = warm_up(&app, &stop).await;
    });
}

#[tauri::command]
pub fn video_tools_cancel(state: State<'_, VideoState>) {
    if let Some(stop) = state.installing.lock().unwrap().take() {
        stop.request();
    }
}

/// «Delete the video tools» (the settings).
#[tauri::command]
pub async fn video_tools_remove(
    app: State<'_, crate::App>,
    state: State<'_, VideoState>,
) -> Result<(), String> {
    // Not while they are being installed or brought up to date.
    let _one_at_a_time = state.tools_lock.lock().await;
    // Off the main thread, where a command that is not async runs: the
    // tools are tens of megabytes, and the windows would wait meanwhile.
    let toolbox = Toolbox::new(folders(&app).0);
    tauri::async_runtime::spawn_blocking(move || toolbox.remove_all())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// A format, for «Video quality».
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct FormatView {
    pub id: String,
    pub ext: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    /// Its size, or the site's guess of it.
    #[ts(type = "number | null")]
    pub size: Option<u64>,
    /// `both`, `video` (picture only) or `audio`.
    pub kind: String,
    pub hls: bool,
    /// The picture's codec as people know it (`H.264`, `VP9`, `AV1`,
    /// `HEVC`), if the site says it: QuickTime plays H.264 and HEVC only
    /// (owner's report, 2026-10-30).
    pub codec: Option<String>,
}

/// A picture codec's name as people know it, from yt-dlp's `vcodec`.
fn codec_name(vcodec: &str) -> Option<&'static str> {
    let v = vcodec.to_ascii_lowercase();
    [
        (["avc1", "h264"], "H.264"),
        (["vp09", "vp9"], "VP9"),
        (["vp08", "vp8"], "VP8"),
        (["av01", "av1"], "AV1"),
        (["hvc1", "hev1"], "HEVC"),
    ]
    .into_iter()
    .find(|(starts, _)| starts.iter().any(|s| v.starts_with(s)))
    .map(|(_, name)| name)
}

/// What to download, as the page says it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ChoiceView {
    Format {
        id: String,
    },
    Merge {
        video: String,
        audio: String,
    },
    AudioOnly,
    /// The engine's best, chosen when it starts and its page is asked (a
    /// playlist's items); then kept as that choice.
    Best,
}

impl From<Choice> for ChoiceView {
    fn from(c: Choice) -> Self {
        match c {
            Choice::Format(id) => ChoiceView::Format { id },
            Choice::Merge { video, audio } => ChoiceView::Merge { video, audio },
            Choice::AudioOnly => ChoiceView::AudioOnly,
        }
    }
}

impl ChoiceView {
    /// The engine's choice, with [`ChoiceView::Best`] chosen among
    /// `formats`.
    fn resolved(&self, formats: &[Format]) -> Result<Choice, VideoErrorView> {
        Ok(match self.clone() {
            ChoiceView::Format { id } => Choice::Format(id),
            ChoiceView::Merge { video, audio } => Choice::Merge { video, audio },
            ChoiceView::AudioOnly => Choice::AudioOnly,
            ChoiceView::Best => zajil_core::video::Video {
                title: String::new(),
                duration: None,
                formats: formats.to_vec(),
            }
            .best_choice()
            .ok_or_else(|| VideoErrorView {
                kind: "no_formats".into(),
                message: "the page offers no format to download".into(),
            })?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct EntryView {
    pub url: Option<String>,
    pub title: Option<String>,
    /// A playlist inside the playlist: opened, not downloaded.
    pub playlist: bool,
}

/// What a page holds.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum VideoInfoView {
    Video {
        title: String,
        duration: Option<f64>,
        formats: Vec<FormatView>,
        best: Option<ChoiceView>,
    },
    Playlist {
        title: String,
        entries: Vec<EntryView>,
    },
}

/// Why it cannot: its kind, for the page's words, and the engine's text
/// (safe to show: no cookie, no token).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct VideoErrorView {
    /// `drm`, `tools_missing`, `refused`, `tool`, `output`, `stopped`,
    /// `io`, or `not_ready` when yt-dlp did not start after its download;
    /// for a failure of yt-dlp's whose words say what it comes to,
    /// `no_formats`, `sign_in`, `private`, `unsupported` or `challenge`;
    /// `playlist` for a playlist's item that is a playlist itself.
    pub kind: String,
    pub message: String,
}

impl From<VideoError> for VideoErrorView {
    fn from(e: VideoError) -> Self {
        let kind = match &e {
            VideoError::Drm => "drm",
            VideoError::Tools(_) => "tools_missing",
            VideoError::Refused(_) => "refused",
            VideoError::Tool(_) => match e.problem() {
                Some(ToolProblem::NoFormats) => "no_formats",
                Some(ToolProblem::SignIn) => "sign_in",
                Some(ToolProblem::Private) => "private",
                Some(ToolProblem::Unsupported) => "unsupported",
                Some(ToolProblem::Challenge) => "challenge",
                None => "tool",
            },
            VideoError::Output(_) => "output",
            VideoError::Stopped => "stopped",
            VideoError::Io(_) => "io",
            VideoError::NotReady(_) => "not_ready",
        };
        VideoErrorView {
            kind: kind.into(),
            message: e.to_string(),
        }
    }
}

impl VideoErrorView {
    /// What the list says of it, in `lang`: a sentence, and yt-dlp's own
    /// words after it when no sentence says what it comes to.
    fn text(&self, lang: zajil_core::settings::Language) -> String {
        let lang = match lang {
            zajil_core::settings::Language::Ar => zajil_i18n::Lang::Ar,
            zajil_core::settings::Language::En => zajil_i18n::Lang::En,
        };
        let localizer = zajil_i18n::Localizer::new(lang);
        let key = match self.kind.as_str() {
            "drm" => "video-drm",
            "tools_missing" => "video-tools-missing",
            "no_formats" => "video-error-no_formats",
            "sign_in" => "video-error-sign_in",
            "private" => "video-error-private",
            "unsupported" => "video-error-unsupported",
            "challenge" => "video-error-challenge",
            "playlist" => "video-error-playlist",
            "not_ready" => "video-tools-not-ready",
            _ => {
                return localizer
                    .text(
                        "video-error-other-details",
                        &[("details", zajil_i18n::Arg::Text(&self.message))],
                    )
                    .unwrap_or_else(|_| self.message.clone());
            }
        };
        localizer
            .text(key, &[])
            .unwrap_or_else(|_| self.message.clone())
    }
}

pub fn format_view(f: &Format) -> FormatView {
    // Codecs the site does not say (X's direct MP4s, a plain file): a
    // picture with its sound when it has a picture's size or a video's
    // extension, as yt-dlp's own «best» takes it (owner's report,
    // 2026-10-29: X offered sound only).
    let unknown = f.vcodec.is_none() && f.acodec.is_none();
    let a_picture = f.height.is_some()
        || f.width.is_some()
        || matches!(
            f.ext.as_str(),
            "mp4" | "webm" | "mov" | "mkv" | "flv" | "3gp"
        );
    let kind = match (f.has_video(), f.has_audio()) {
        (true, true) => "both",
        (true, false) => "video",
        _ if unknown && a_picture => "both",
        _ => "audio",
    };
    FormatView {
        id: f.id.clone(),
        ext: f.ext.clone(),
        width: f.width,
        height: f.height,
        fps: f.fps,
        size: f.filesize.or(f.filesize_approx),
        kind: kind.into(),
        hls: f.is_hls(),
        codec: f.vcodec.as_deref().and_then(codec_name).map(Into::into),
    }
}

#[tauri::command]
pub async fn video_probe(
    app: State<'_, crate::App>,
    state: State<'_, VideoState>,
    url: String,
) -> Result<VideoInfoView, VideoErrorView> {
    let stop = Stop::new();
    if let Some(old) = state.probing.lock().unwrap().replace(stop.clone()) {
        old.request();
    }
    let cookies = state.cookies.lock().unwrap().get(&url).cloned();
    let media = state.media.lock().unwrap().get(&url).cloned();
    // Read ahead while the pointer was on it: taken, or waited for.
    let ahead = state
        .ahead
        .lock()
        .unwrap()
        .find(&url, cookies.is_some(), Instant::now());
    let read = match ahead {
        Some(mut ahead) => tokio::select! {
            read = ahead.wait_for(Option::is_some) => read
                .ok()
                .and_then(|r| r.clone().flatten())
                .filter(|sent| ahead_fits(sent, &url, media.as_deref())),
            () = stop.requested() => return Err(VideoError::Stopped.into()),
        },
        None => None,
    };
    let sent = match read {
        Some(sent) => sent,
        None => {
            runner(&app)
                .await
                .probe_sent(&url, media.as_deref(), cookies.as_deref(), &stop)
                .await?
        }
    };
    state.probing.lock().unwrap().take();
    if !sent.cookies_kept {
        state.cookies.lock().unwrap().remove(&url);
    }
    // Read by the video's own link: the download goes from that one.
    if sent.url != url {
        state
            .read_as
            .lock()
            .unwrap()
            .insert(url.clone(), sent.url.clone());
    }
    // Never the links (they may carry a token): which was read, and what.
    if let Probed::Video(video) = &sent.probed {
        let pictures = video
            .formats
            .iter()
            .filter(|f| format_view(f).kind != "audio")
            .count();
        eprintln!(
            "zajil: read from {}: {} formats, {pictures} with a picture",
            if sent.url == url {
                "the page"
            } else {
                "the page's playlist"
            },
            video.formats.len(),
        );
        // Each format's height and codecs, never its link (owner's
        // reports, 2026-10-30: which picture a site offers).
        let each: Vec<String> = video
            .formats
            .iter()
            .map(|f| {
                format!(
                    "{}p {} {}/{}",
                    f.height.unwrap_or(0),
                    f.ext,
                    f.vcodec.as_deref().unwrap_or("?"),
                    f.acodec.as_deref().unwrap_or("?")
                )
            })
            .collect();
        eprintln!("zajil: the formats: {}", each.join(", "));
    }
    let read = sent.url;
    Ok(match sent.probed {
        Probed::Video(video) => {
            let best = video.best_choice().map(ChoiceView::from);
            let formats = video.formats.iter().map(format_view).collect();
            state
                .formats
                .lock()
                .unwrap()
                .insert(read, video.formats.clone());
            VideoInfoView::Video {
                title: video.title,
                duration: video.duration,
                formats,
                best,
            }
        }
        Probed::Playlist { title, entries } => VideoInfoView::Playlist {
            title,
            entries: entries
                .into_iter()
                .map(|e| EntryView {
                    playlist: e.is_playlist(),
                    url: e.url,
                    title: e.title,
                })
                .collect(),
        },
    })
}

/// The extension's button showed over a video: its page is read now, in
/// the background, unless it was read in the last ten minutes ([`Aheads`]).
/// Nothing is shown, and nothing of it is kept but in memory.
pub fn read_ahead<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    page: String,
    media: Option<String>,
    cookies: Option<String>,
) {
    let begun = handle.state::<VideoState>().ahead.lock().unwrap().begin(
        &page,
        cookies.is_some(),
        Instant::now(),
    );
    let Some(done) = begun else {
        return;
    };
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let runner = runner(&handle.state::<crate::App>()).await;
        let read = runner
            .probe_sent(&page, media.as_deref(), cookies.as_deref(), &Stop::new())
            .await
            .ok();
        let _ = done.send(Some(read));
    });
}

/// Asks `url`'s page, with its cookies from the browser if any, else
/// without them ([`VideoRunner::probe_with_or_without_cookies`]). Cookies
/// that left no format are forgotten, so the download goes the same way.
async fn probe(
    state: &VideoState,
    runner: &VideoRunner,
    url: &str,
    stop: &Stop,
) -> Result<Probed, VideoError> {
    let cookies = state.cookies.lock().unwrap().get(url).cloned();
    let (probed, kept) = runner
        .probe_with_or_without_cookies(url, cookies.as_deref(), stop)
        .await?;
    if !kept {
        state.cookies.lock().unwrap().remove(url);
    }
    Ok(probed)
}

#[tauri::command]
pub fn video_probe_cancel(state: State<'_, VideoState>) {
    if let Some(stop) = state.probing.lock().unwrap().take() {
        stop.request();
    }
}

#[derive(Debug, Clone, Serialize)]
struct Progress {
    id: u64,
    done: u64,
    total: Option<u64>,
    speed: Option<u64>,
}

/// Downloads a probed video into `dest` as `name`: it joins the list, and
/// its id comes at once; then `video-progress`, and `video-finished` or
/// `video-failed`.
#[tauri::command]
pub fn video_download<R: tauri::Runtime>(
    app: State<'_, crate::App>,
    state: State<'_, VideoState>,
    url: String,
    choice: ChoiceView,
    dest: String,
    name: String,
    handle: tauri::AppHandle<R>,
) -> Result<u64, String> {
    let id = FIRST_ID + state.next.fetch_add(1, Ordering::SeqCst) + 1;
    let private = state.private.lock().unwrap().contains(&url);
    let url = state
        .read_as
        .lock()
        .unwrap()
        .get(&url)
        .cloned()
        .unwrap_or(url);
    state.rows.lock().unwrap().push(Row {
        id,
        url,
        name,
        folder: dest,
        choice,
        status: StatusView::Waiting,
        size: None,
        done: 0,
        speed: 0,
        error: None,
        file: None,
        private,
        move_to: None,
        label: None,
        added: Some(zajil_core::state::now_rfc3339()),
    });
    state.fresh.lock().unwrap().insert(id);
    save_rows(&app, &state);
    let _ = handle.emit("downloads-changed", ());
    pump(&handle);
    Ok(id)
}

/// A video of a playlist, for [`video_add_many`].
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct NewVideo {
    pub url: String,
    pub title: Option<String>,
}

/// A playlist's chosen videos join the list at once, each at its best,
/// named after its title; they start in turn, at most the settings' number at
/// a time, and each asks its own page then. Those on the playlist's site
/// take its cookies, if the browser sent any. Returns their ids.
#[tauri::command]
pub fn video_add_many<R: tauri::Runtime>(
    app: State<'_, crate::App>,
    state: State<'_, VideoState>,
    page: String,
    items: Vec<NewVideo>,
    dest: String,
    handle: tauri::AppHandle<R>,
) -> Vec<u64> {
    let page_cookies = state.cookies.lock().unwrap().get(&page).cloned();
    let private = state.private.lock().unwrap().contains(&page);
    let mut ids = Vec::new();
    for item in items {
        if let Some(cookies) = &page_cookies
            && same_site(&page, &item.url)
        {
            state
                .cookies
                .lock()
                .unwrap()
                .entry(item.url.clone())
                .or_insert_with(|| cookies.clone());
        }
        let id = FIRST_ID + state.next.fetch_add(1, Ordering::SeqCst) + 1;
        let mut row = new_row(id, item, &dest);
        row.private = private;
        state.rows.lock().unwrap().push(row);
        ids.push(id);
    }
    save_rows(&app, &state);
    let _ = handle.emit("downloads-changed", ());
    pump(&handle);
    ids
}

/// Whether two links are on the same host: a playlist's cookies go only
/// to its own site's videos (ADR-0005).
fn same_site(a: &str, b: &str) -> bool {
    crate::view::host_of(a).is_some_and(|host| crate::view::host_of(b) == Some(host))
}

/// A playlist's video as a waiting row: at its best, named after its
/// title as a file can be.
fn new_row(id: u64, item: NewVideo, dest: &str) -> Row {
    let name = item
        .title
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .map_or_else(|| "video".into(), zajil_core::sanitize::sanitize_file_name);
    Row {
        id,
        url: item.url,
        name,
        folder: dest.to_owned(),
        choice: ChoiceView::Best,
        status: StatusView::Waiting,
        size: None,
        done: 0,
        speed: 0,
        error: None,
        file: None,
        private: false,
        move_to: None,
        label: None,
        added: Some(zajil_core::state::now_rfc3339()),
    }
}

/// Runs video download `id` of the list; `ask` asks its page first (a
/// resumed one: the formats of the last probe may be gone, or stale).
fn start<R: tauri::Runtime>(handle: tauri::AppHandle<R>, id: u64, ask: bool) {
    let state = handle.state::<VideoState>();
    let Some(mut row) = with_row(&state, id, |row| {
        row.status = StatusView::Active;
        row.clone()
    }) else {
        return;
    };
    let stop = Stop::new();
    state.downloads.lock().unwrap().insert(id, stop.clone());
    let known = state.formats.lock().unwrap().get(&row.url).cloned();
    // What was read ahead (the browser's button showed over it), if it fits.
    let ahead = {
        let cookies = state.cookies.lock().unwrap().contains_key(&row.url);
        let media = state.media.lock().unwrap().get(&row.url).cloned();
        state
            .ahead
            .lock()
            .unwrap()
            .find(&row.url, cookies, Instant::now())
            .map(|read| (read, media))
    };
    tauri::async_runtime::spawn(async move {
        let runner = runner(&handle.state::<crate::App>()).await;
        let events = handle.clone();
        let mut on_event = move |event: VideoEvent| match event {
            VideoEvent::Progress { done, total, speed } => {
                progressed(&events, id, done, total, speed);
            }
            // A part saved before was of another file: said in the list.
            VideoEvent::StartedOver(why) => {
                eprintln!("zajil: video {id}: {why}");
                with_row(&events.state::<VideoState>(), id, |row| {
                    row.error = Some(why)
                });
            }
        };
        // Stopped before, Zajil's own way: continued from its parts with
        // the links saved there, without asking the page again (no yt-dlp,
        // which YouTube may stop without the browser's cookies).
        let saved_ids = match &row.choice {
            ChoiceView::Format { id } => vec![id.clone()],
            ChoiceView::Merge { video, audio } => vec![video.clone(), audio.clone()],
            _ => Vec::new(),
        };
        let resumed = if ask && !row.name.is_empty() && !saved_ids.is_empty() {
            let saved = zajil_core::video::Saved {
                page: &row.url,
                ids: &saved_ids,
                dest: std::path::Path::new(&row.folder),
                name: &row.name,
            };
            runner.resume_saved(&saved, &stop, &mut on_event).await
        } else {
            None
        };
        let result = match resumed {
            Some(done) => done.map_err(VideoErrorView::from),
            None => {
                async {
                    let read = match ahead {
                        Some((mut read, media)) => tokio::select! {
                            read = read.wait_for(Option::is_some) => read
                                .ok()
                                .and_then(|r| r.clone().flatten())
                                .filter(|sent| ahead_fits(sent, &row.url, media.as_deref())),
                            () = stop.requested() => None,
                        },
                        None => None,
                    };
                    let probed = match read {
                        Some(sent) => Ok(sent.probed),
                        None if known.is_some() && !ask => Err(None),
                        None => probe(
                            handle.state::<VideoState>().inner(),
                            &runner,
                            &row.url,
                            &stop,
                        )
                        .await
                        .map_err(Some),
                    };
                    let formats = match probed {
                        Err(None) => Ok(known.unwrap_or_default()),
                        Err(Some(e)) => Err(e.into()),
                        Ok(Probed::Video(video)) => {
                            // From the browser's list: named after its title, and
                            // its choice in yt-dlp's names.
                            if row.name.is_empty() {
                                let name = Some(video.title.trim())
                                    .filter(|t| !t.is_empty())
                                    .map_or_else(
                                        || "video".into(),
                                        zajil_core::sanitize::sanitize_file_name,
                                    );
                                let choice = from_list(&row.choice, &video.formats);
                                with_row(&handle.state::<VideoState>(), id, |r| {
                                    r.name = name.clone();
                                    r.choice = choice.clone();
                                });
                                row.name = name;
                                row.choice = choice;
                            }
                            Ok(video.formats)
                        }
                        Ok(Probed::Playlist { .. }) => Err(VideoErrorView {
                            kind: "playlist".into(),
                            message: "the page is a playlist".into(),
                        }),
                    };
                    // The engine's best, once chosen, is the row's choice from now on.
                    let chosen = formats.and_then(|formats| {
                        let wanted = still_offered(&row.choice, &formats);
                        if wanted != row.choice {
                            eprintln!("zajil: video {id}: its format is gone, the best goes");
                            row.choice = wanted;
                        }
                        let choice = row.choice.resolved(&formats)?;
                        if row.choice == ChoiceView::Best {
                            with_row(&handle.state::<VideoState>(), id, |row| {
                                row.choice = ChoiceView::from(choice.clone());
                            });
                        }
                        Ok((formats, choice))
                    });
                    // After the probe: the cookies it went with, if any.
                    let cookies = handle
                        .state::<VideoState>()
                        .cookies
                        .lock()
                        .unwrap()
                        .get(&row.url)
                        .cloned();
                    match chosen {
                        Ok((formats, choice)) => {
                            let request = Download {
                                url: &row.url,
                                choice: &choice,
                                formats: &formats,
                                cookies: cookies.as_deref(),
                                dest: std::path::Path::new(&row.folder),
                                name: &row.name,
                            };
                            runner
                                .download(&request, &stop, &mut on_event)
                                .await
                                .map_err(VideoErrorView::from)
                        }
                        Err(e) => Err(e),
                    }
                }
                .await
            }
        };
        let state = handle.state::<VideoState>();
        state.downloads.lock().unwrap().remove(&id);
        state.told.lock().unwrap().remove(&id);
        // Cancelled while it ran: gone, with what it wrote.
        if state.discard.lock().unwrap().remove(&id) {
            let mut gone = row.clone();
            gone.file = result.as_ref().ok().map(|f| f.display().to_string());
            state.rows.lock().unwrap().retain(|r| r.id != id);
            save_rows(&handle.state::<crate::App>(), &state);
            let _ = handle.emit("downloads-changed", ());
            pump(&handle);
            let _ = tauri::async_runtime::spawn_blocking(move || remove_leftovers(&gone)).await;
            return;
        }
        // Finished where it downloaded: moved where the user said meanwhile.
        let move_to = with_row(&state, id, |row| {
            // Stopped: kept for when it finishes.
            if result.is_err() {
                return None;
            }
            row.move_to
                .take()
                .filter(|to| !row.lands_at(to))
                .map(|mut to| {
                    if to.name.is_empty() {
                        to.name = row.name.clone();
                    }
                    to
                })
        })
        .flatten();
        let result = match (result, move_to) {
            (Ok(file), Some(to)) => {
                let target = to.clone();
                let moved = tauri::async_runtime::spawn_blocking(move || {
                    zajil_core::video::move_video(
                        &file,
                        std::path::Path::new(&target.folder),
                        &target.name,
                    )
                    .map_err(|e| (e, file))
                })
                .await;
                match moved {
                    Ok(Ok(file)) => {
                        with_row(&state, id, |row| row.folder = to.folder.clone());
                        Ok(file)
                    }
                    // Left where it is, and said.
                    Ok(Err((e, file))) => {
                        eprintln!("zajil: video {id} could not be moved: {e}");
                        Ok(file)
                    }
                    Err(e) => Err(VideoErrorView {
                        kind: "io".into(),
                        message: e.to_string(),
                    }),
                }
            }
            (result, _) => result,
        };
        let lang = handle
            .state::<crate::App>()
            .settings
            .load()
            .map(|s| s.language)
            .unwrap_or_default();
        with_row(&state, id, |row| match &result {
            Ok(file) => {
                row.status = StatusView::Done;
                row.error = None;
                row.file = Some(file.display().to_string());
                row.size = std::fs::metadata(file).ok().map(|m| m.len()).or(row.size);
                row.done = row.size.unwrap_or(row.done);
            }
            Err(e) if e.kind == "stopped" => {
                row.status = StatusView::Paused;
            }
            Err(e) => {
                // Its kind only: yt-dlp's words may hold the page's link.
                eprintln!("zajil: video {id} failed ({})", e.kind);
                row.status = StatusView::Failed;
                row.error = Some(e.text(lang));
            }
        });
        save_rows(&handle.state::<crate::App>(), &state);
        let _ = handle.emit("downloads-changed", ());
        // Its place is free: the next waiting one starts.
        pump(&handle);
        match result {
            Ok(file) => {
                let _ = handle.emit(
                    "video-finished",
                    serde_json::json!({ "id": id, "file": file.display().to_string() }),
                );
            }
            Err(error) => {
                let _ = handle.emit(
                    "video-failed",
                    serde_json::json!({ "id": id, "error": error }),
                );
            }
        }
    });
}

/// A running video's progress: to «Video quality» each time, and to the
/// list at most four times a second.
fn progressed<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    id: u64,
    done: u64,
    total: Option<u64>,
    speed: Option<u64>,
) {
    let _ = handle.emit(
        "video-progress",
        Progress {
            id,
            done,
            total,
            speed,
        },
    );
    let state = handle.state::<VideoState>();
    with_row(&state, id, |row| {
        row.done = done;
        row.size = total;
        row.speed = speed.unwrap_or(0);
    });
    let now = Instant::now();
    let mut told = state.told.lock().unwrap();
    if told
        .get(&id)
        .is_none_or(|at| now.duration_since(*at) >= Duration::from_millis(250))
    {
        told.insert(id, now);
        let _ = handle.emit("downloads-changed", ());
    }
}

/// Stops a video download (yt-dlp and what it started end).
#[tauri::command]
pub fn video_cancel(state: State<'_, VideoState>, id: u64) {
    if let Some(stop) = state.downloads.lock().unwrap().remove(&id) {
        stop.request();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A choice from the browser's list names YouTube's itags; yt-dlp's
    /// formats may add a suffix, and a "stable volume" sound is not taken.
    #[test]
    fn a_choice_from_the_list_finds_yt_dlps_format() {
        let format = |id: &str| -> Format {
            serde_json::from_value(serde_json::json!({ "format_id": id })).unwrap()
        };
        let formats = [
            format("299"),
            format("140-drc"),
            format("140-1"),
            format("251"),
        ];
        assert_eq!(
            from_list(
                &ChoiceView::Merge {
                    video: "299".into(),
                    audio: "140".into()
                },
                &formats
            ),
            ChoiceView::Merge {
                video: "299".into(),
                audio: "140-1".into()
            }
        );
        assert_eq!(
            from_list(&ChoiceView::Format { id: "251".into() }, &formats),
            ChoiceView::Format { id: "251".into() }
        );
        assert_eq!(
            from_list(&ChoiceView::Format { id: "18".into() }, &formats),
            ChoiceView::Best
        );
    }

    /// Closing «Video quality» stops its check of the page and its tools'
    /// download (the owner's notes on 1.0: they ran on behind it).
    #[test]
    fn closing_the_window_gives_up_its_check_and_its_tools_download() {
        let state = VideoState::default();
        let (probing, installing) = (Stop::new(), Stop::new());
        *state.probing.lock().unwrap() = Some(probing.clone());
        *state.installing.lock().unwrap() = Some(installing.clone());
        give_up(&state);
        assert!(probing.is_requested() && installing.is_requested());
        assert!(
            state.probing.lock().unwrap().is_none() && state.installing.lock().unwrap().is_none()
        );
        // Nothing running: nothing to stop.
        give_up(&state);
    }

    fn sent(page: &str) -> zajil_core::video::Sent {
        zajil_core::video::Sent {
            probed: Probed::Playlist {
                title: String::new(),
                entries: Vec::new(),
            },
            url: page.into(),
            cookies_kept: false,
        }
    }

    /// A video is read ahead once in ten minutes, two at a time at most,
    /// and twenty are kept at most, the oldest going first (owner,
    /// 2026-10-28).
    #[test]
    fn a_video_is_read_ahead_once_in_ten_minutes_and_two_at_a_time() {
        let mut aheads = Aheads::default();
        let now = Instant::now();
        let a = aheads.begin("https://a.example/", true, now).unwrap();
        assert!(aheads.begin("https://a.example/", true, now).is_none());
        let b = aheads.begin("https://b.example/", false, now).unwrap();
        // Two run: a third waits for a later pass of the pointer.
        assert!(aheads.begin("https://c.example/", false, now).is_none());
        a.send(Some(None)).unwrap();
        assert!(aheads.begin("https://c.example/", false, now).is_some());
        // Read, even if it failed: not again before ten minutes.
        assert!(aheads.begin("https://a.example/", true, now).is_none());
        b.send(Some(Some(sent("https://b.example/")))).unwrap();
        let later = now + AHEAD_FOR;
        assert!(aheads.begin("https://a.example/", true, later).is_some());
        assert!(!aheads.0.contains_key("https://b.example/"));

        let mut aheads = Aheads::default();
        for i in 0..=AHEAD_KEPT {
            let at = now + Duration::from_secs(i as u64);
            let done = aheads.begin(&format!("https://{i}.example/"), false, at);
            done.unwrap().send(Some(None)).unwrap();
        }
        assert_eq!(aheads.0.len(), AHEAD_KEPT);
        assert!(!aheads.0.contains_key("https://0.example/"));
    }

    /// The press takes a read of its page, made with cookies if it has
    /// some and without if not, from the last ten minutes; it waits for one
    /// still running.
    #[test]
    fn the_press_takes_the_read_of_its_own_page_or_waits_for_it() {
        let mut aheads = Aheads::default();
        let now = Instant::now();
        let page = "https://www.youtube.com/watch?v=x";
        let done = aheads.begin(page, true, now).unwrap();
        assert!(aheads.find(page, false, now).is_none());
        assert!(
            aheads
                .find("https://www.youtube.com/watch?v=y", true, now)
                .is_none()
        );
        let mut read = aheads.find(page, true, now).unwrap();
        let waited = tauri::async_runtime::block_on(async move {
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                done.send(Some(Some(sent(page)))).unwrap();
            });
            read.wait_for(Option::is_some).await.unwrap().clone()
        });
        assert_eq!(waited, Some(Some(sent(page))));
        assert!(aheads.find(page, true, now + AHEAD_FOR).is_none());
    }

    /// A read of the page fits a press whatever video link it has (on
    /// YouTube it changes between the pointer's pass and the press); a
    /// read of the video's own link fits only a press with that link.
    #[test]
    fn a_read_of_the_page_fits_any_press_on_it_and_one_of_the_link_only_its_own() {
        let page = "https://www.youtube.com/watch?v=x";
        assert!(ahead_fits(&sent(page), page, None));
        assert!(ahead_fits(
            &sent(page),
            page,
            Some("https://cdn.example/other.m3u8")
        ));
        let media = "https://cdn.example/v.m3u8";
        assert!(ahead_fits(&sent(media), page, Some(media)));
        assert!(!ahead_fits(
            &sent(media),
            page,
            Some("https://cdn.example/w.m3u8")
        ));
        assert!(!ahead_fits(&sent(media), page, None));
    }

    /// «About» names each tool's source: its pinned release, or its
    /// project.
    #[test]
    fn every_tool_has_its_source() {
        let pin = zajil_core::tools::Pinned {
            version: "2026.10.10".into(),
            url: "https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.10/yt-dlp_macos"
                .into(),
            sha256: String::new(),
            archive: None,
        };
        assert_eq!(
            source_of(Tool::YtDlp, Some(&pin)),
            "https://github.com/yt-dlp/yt-dlp/releases/tag/2026.10.10"
        );
        for tool in TOOLS {
            assert!(source_of(tool, None).starts_with("https://github.com/"));
        }
    }

    fn row(id: u64, status: StatusView) -> Row {
        Row {
            id,
            url: "https://videos.example.org/watch/1?t=secret".into(),
            name: "درس".into(),
            folder: "/tmp".into(),
            choice: ChoiceView::Merge {
                video: "137".into(),
                audio: "140".into(),
            },
            status,
            size: Some(100),
            done: 40,
            speed: 9,
            error: None,
            file: None,
            private: false,
            move_to: None,
            label: None,
            added: None,
        }
    }

    /// Videos' ids never meet the queues', and stay exact in JavaScript.
    #[test]
    fn video_ids_are_their_own() {
        assert!(!is_video(1) && !is_video(FIRST_ID - 1) && is_video(FIRST_ID + 1));
        const { assert!(FIRST_ID * 2 <= 1 << 53) };
        let (videos, queues) = split(vec![3, FIRST_ID + 2, 7]);
        assert_eq!((videos, queues), (vec![FIRST_ID + 2], vec![3, 7]));
    }

    /// After a restart, a video that ran is paused with what came (Zajil's
    /// own way continues from it); a file changed by hand never takes a
    /// queue's id.
    #[test]
    fn the_list_comes_back_paused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(ROWS_FILE);
        let rows = vec![
            row(FIRST_ID + 1, StatusView::Active),
            row(FIRST_ID + 2, StatusView::Done),
            row(5, StatusView::Done),
        ];
        std::fs::write(&path, serde_json::to_vec(&rows).unwrap()).unwrap();
        let back = read_rows(&path);
        assert_eq!(back.len(), 2);
        assert_eq!((back[0].status, back[0].done), (StatusView::Paused, 40));
        assert_eq!((back[1].status, back[1].done), (StatusView::Done, 40));
        // The speed is not kept.
        assert_eq!(back[1].speed, 0);
        std::fs::write(&path, "not json").unwrap();
        assert!(read_rows(&path).is_empty());
    }

    /// A video's row: the host only, the videos' category (the sounds' for
    /// «audio only»), no queue, and a speed only while it runs.
    #[test]
    fn a_video_row_in_the_list() {
        let categories = zajil_core::category::CategoriesFile::built_in();
        let view = view_of(&row(FIRST_ID + 1, StatusView::Active), &categories);
        assert_eq!(view.host.as_deref(), Some("videos.example.org"));
        assert_eq!(view.name, "درس");
        assert!(view.video && view.queue.is_empty() && view.parts.is_empty());
        assert_eq!(view.speed, 9.0);
        let video = view.category.clone();
        assert!(video.is_some());
        let mut audio = row(FIRST_ID + 2, StatusView::Paused);
        audio.choice = ChoiceView::AudioOnly;
        let audio = view_of(&audio, &categories);
        assert_eq!(audio.speed, 0.0);
        assert!(audio.category.is_some() && audio.category != video);
        let mut done = row(FIRST_ID + 3, StatusView::Done);
        done.file = Some("/tmp/درس.mp4".into());
        assert_eq!(view_of(&done, &categories).name, "درس.mp4");
    }

    /// The page's names for what can go wrong, one per kind.
    #[test]
    fn every_error_has_its_kind() {
        let kinds: Vec<String> = [
            VideoError::Drm,
            VideoError::Refused("no".into()),
            VideoError::Tool("x".into()),
            VideoError::Output("x".into()),
            VideoError::Stopped,
            VideoError::Io(std::io::Error::other("x")),
        ]
        .into_iter()
        .map(|e| VideoErrorView::from(e).kind)
        .collect();
        assert_eq!(kinds, ["drm", "refused", "tool", "output", "stopped", "io"]);
    }

    /// A stopped download whose format this read no longer offers (an HLS
    /// one, numbered anew) goes at the engine's best; one still offered,
    /// or with nothing read, as chosen.
    #[test]
    fn a_format_gone_since_goes_at_the_best() {
        let f = |id: &str| -> Format {
            serde_json::from_value(
                serde_json::json!({ "format_id": id, "vcodec": "avc1", "acodec": "mp4a" }),
            )
            .unwrap()
        };
        let read = [f("96-1"), f("137"), f("140")];
        let format = |id: &str| ChoiceView::Format { id: id.into() };
        assert_eq!(still_offered(&format("96-2"), &read), ChoiceView::Best);
        assert_eq!(still_offered(&format("96-1"), &read), format("96-1"));
        let merge = ChoiceView::Merge {
            video: "137".into(),
            audio: "140".into(),
        };
        assert_eq!(still_offered(&merge, &read), merge);
        assert_eq!(still_offered(&format("96-2"), &[]), format("96-2"));
    }

    /// «Delete» with the unfinished files: only the unfinished go with
    /// their parts; a finished one, or one not in the list, keeps its file.
    #[test]
    fn deleting_with_files_spares_the_finished() {
        let row = |id, status| Row {
            status,
            ..new_row(
                id,
                NewVideo {
                    url: "https://example.com/v".into(),
                    title: None,
                },
                "/tmp",
            )
        };
        let rows = [
            row(1, StatusView::Paused),
            row(2, StatusView::Done),
            row(3, StatusView::Failed),
            row(4, StatusView::Active),
        ];
        assert_eq!(
            by_finished(&rows, &[1, 2, 3, 4, 9]),
            (vec![1, 3, 4], vec![2, 9])
        );
    }

    /// A format as the page sees it: its kind from its codecs, its size
    /// or the site's guess, and whether it is HLS.
    #[test]
    fn formats_say_their_kind_and_size() {
        let f = |json: &str| format_view(&serde_json::from_str::<Format>(json).unwrap());
        let both = f(
            r#"{"format_id":"22","ext":"mp4","width":1280,"height":720,"vcodec":"avc1","acodec":"mp4a","filesize":1000}"#,
        );
        assert_eq!(
            (both.kind.as_str(), both.size, both.height),
            ("both", Some(1000), Some(720))
        );
        let picture = f(
            r#"{"format_id":"137","ext":"mp4","vcodec":"avc1","acodec":"none","filesize_approx":5000}"#,
        );
        assert_eq!((picture.kind.as_str(), picture.size), ("video", Some(5000)));
        let sound = f(r#"{"format_id":"140","ext":"m4a","vcodec":"none","acodec":"mp4a"}"#);
        assert_eq!((sound.kind.as_str(), sound.size), ("audio", None));
        let hls = f(
            r#"{"format_id":"hls-720","ext":"mp4","vcodec":"avc1","acodec":"mp4a","protocol":"m3u8_native"}"#,
        );
        assert!(hls.hls);
        assert_eq!(both.codec.as_deref(), Some("H.264"));
        let vp9 =
            f(r#"{"format_id":"dash-1v","ext":"mp4","vcodec":"vp09.00.51.08","acodec":"none"}"#);
        assert_eq!(vp9.codec.as_deref(), Some("VP9"));
        assert_eq!(sound.codec, None);
        // X's direct MP4: no codecs said, a picture's size.
        let x = f(r#"{"format_id":"http-2176","ext":"mp4","width":1280,"height":720}"#);
        assert_eq!((x.kind.as_str(), x.height), ("both", Some(720)));
        let plain = f(r#"{"format_id":"0","ext":"mp4"}"#);
        assert_eq!(plain.kind, "both");
        // Nothing said, and no picture's sign: sound.
        let mp3 = f(r#"{"format_id":"0","ext":"mp3"}"#);
        assert_eq!(mp3.kind, "audio");
    }

    /// What the page sends comes back as the engine's choice, and back.
    #[test]
    fn choices_go_both_ways() {
        for choice in [
            Choice::Format("22".into()),
            Choice::Merge {
                video: "137".into(),
                audio: "140".into(),
            },
            Choice::AudioOnly,
        ] {
            assert_eq!(ChoiceView::from(choice.clone()).resolved(&[]), Ok(choice));
        }
        let json: ChoiceView =
            serde_json::from_str(r#"{"kind":"merge","video":"137","audio":"140"}"#).unwrap();
        assert_eq!(
            json,
            ChoiceView::Merge {
                video: "137".into(),
                audio: "140".into()
            }
        );
    }

    /// A failed video says in the list what it comes to, in the user's
    /// language; yt-dlp's words follow only when no sentence names it.
    #[test]
    fn a_failure_is_told_in_the_user_s_language() {
        use zajil_core::settings::Language;
        let known = VideoErrorView::from(VideoError::Tool(
            "[youtube] abc: Sign in to confirm your age".into(),
        ));
        assert_eq!(known.kind, "sign_in");
        let text = known.text(Language::Ar);
        assert!(text.contains("تسجيل الدخول"), "{text}");
        assert!(!text.contains("Sign in"), "{text}");
        let other = VideoErrorView::from(VideoError::Tool("HTTP Error 500".into()));
        assert_eq!(other.kind, "tool");
        let text = other.text(Language::Ar);
        assert!(
            text.contains("تعذّر تحميل هذا الفيديو") && text.contains("HTTP Error 500"),
            "{text}"
        );
        assert!(
            VideoErrorView::from(VideoError::Drm)
                .text(Language::En)
                .contains("protected")
        );
    }

    fn item(url: &str, title: Option<&str>) -> NewVideo {
        NewVideo {
            url: url.into(),
            title: title.map(Into::into),
        }
    }

    /// A playlist's chosen videos all join the list at once, and at most
    /// the settings' number run: the rest wait their turn, first come
    /// first.
    #[test]
    fn a_playlist_s_videos_wait_their_turn() {
        let mut rows: Vec<Row> = (1..=5)
            .map(|n| new_row(FIRST_ID + n, item("https://example.com/v", Some("v")), "/d"))
            .collect();
        let none = HashSet::new();
        assert_eq!(to_start(&rows, &none, 2), [FIRST_ID + 1, FIRST_ID + 2]);
        assert_eq!(to_start(&rows, &none, 3).len(), 3);
        let two: HashSet<u64> = [FIRST_ID + 1, FIRST_ID + 2].into();
        for row in &mut rows[..2] {
            row.status = StatusView::Active;
        }
        assert!(to_start(&rows, &two, 2).is_empty());
        // One ended, one paused: the next waiting one, not the paused.
        rows[0].status = StatusView::Done;
        rows[2].status = StatusView::Paused;
        let one: HashSet<u64> = [FIRST_ID + 2].into();
        assert_eq!(to_start(&rows, &one, 2), [FIRST_ID + 4]);
    }

    /// Each one goes at the engine's best, named after its title as a
    /// file can be, and keeps that choice once its page is asked.
    #[test]
    fn a_playlist_s_video_goes_at_its_best_under_its_title() {
        let row = new_row(
            FIRST_ID + 1,
            item("https://example.com/v", Some("a/b: c")),
            "/d",
        );
        assert_eq!(row.choice, ChoiceView::Best);
        assert_eq!(row.status, StatusView::Waiting);
        assert!(
            !row.name.contains('/') && row.name.contains('c'),
            "{}",
            row.name
        );
        assert_eq!(
            new_row(FIRST_ID + 2, item("https://e.com/w", None), "/d").name,
            "video"
        );

        let format = |id: &str, height: u32, vcodec: &str, acodec: &str| {
            serde_json::from_value::<Format>(serde_json::json!({
                "format_id": id, "height": height, "vcodec": vcodec, "acodec": acodec
            }))
            .unwrap()
        };
        let formats = [
            format("18", 360, "avc1", "mp4a"),
            format("137", 1080, "avc1", "none"),
            format("140", 0, "none", "mp4a"),
        ];
        assert_eq!(
            ChoiceView::Best.resolved(&formats),
            Ok(Choice::Merge {
                video: "137".into(),
                audio: "140".into()
            })
        );
        assert_eq!(
            ChoiceView::Best.resolved(&[]).unwrap_err().kind,
            "no_formats"
        );
    }

    /// A playlist's cookies go to its own site's videos only.
    #[test]
    fn a_playlist_s_cookies_stay_on_its_site() {
        let page = "https://www.youtube.com/playlist?list=PLx";
        assert!(same_site(page, "https://www.youtube.com/watch?v=abc"));
        assert!(!same_site(page, "https://youtu.be.example.com/watch?v=abc"));
        assert!(!same_site(page, "not a link"));
        assert!(!same_site("not a link", "not a link"));
    }

    /// A private window's video is in the list, but never in
    /// `videos.json` (zajil-host: Zajil keeps nothing of it).
    #[test]
    fn a_private_window_s_video_is_never_written_down() {
        let mut secret = row(FIRST_ID + 2, StatusView::Active);
        secret.private = true;
        secret.url = "https://private.example.org/watch/2".into();
        let rows = [row(FIRST_ID + 1, StatusView::Done), secret];
        let saved = serde_json::to_string(&kept(&rows)).unwrap();
        assert!(!saved.contains("private.example.org"), "{saved}");
        assert!(saved.contains("videos.example.org"), "{saved}");
    }
}
