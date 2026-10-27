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
use std::sync::atomic::{AtomicU64, Ordering};
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
}

/// At most this many video downloads run at once; the others wait in the
/// list (a playlist's items, many at a time).
const MAX_RUNNING: usize = 2;

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
        if matches!(row.status, StatusView::Active | StatusView::Waiting) {
            row.status = StatusView::Paused;
            row.done = 0;
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
    let rows = state.rows.lock().unwrap();
    let row = rows.iter().find(|r| r.id == id)?;
    Some(DetailsView {
        download: view_of(row, categories),
        url: row.url.clone(),
        segments: Vec::new(),
        // A stopped video starts over.
        resumable: Some(false),
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
                row.done = 0;
                row.size = None;
            }
        });
    }
    save_rows(&handle.state::<crate::App>(), &state);
    let _ = handle.emit("downloads-changed", ());
    pump(handle);
}

/// The waiting video downloads that may start now, first come first: as
/// many as leave at most [`MAX_RUNNING`] running.
fn to_start(rows: &[Row], running: &HashSet<u64>) -> Vec<u64> {
    let free = MAX_RUNNING.saturating_sub(running.len());
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
    let ids = to_start(&state.rows.lock().unwrap(), &running);
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
/// and «Video quality» opened for its page.
pub fn receive<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    url: String,
    media: Option<String>,
    cookies: Option<String>,
    private: bool,
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
    *state.pending.lock().unwrap() = Some(url);
    let _ = open_window(handle);
}

/// Opens «Video quality» (or brings it to the front), titled in the
/// page's language by the page itself.
pub fn open_window<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) -> tauri::Result<()> {
    crate::windows::open(handle, LABEL, "Zajil", SIZE)
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

fn tools_status(toolbox: &Toolbox) -> Vec<ToolView> {
    let target = zajil_platform::target::current();
    TOOLS
        .iter()
        .map(|&tool| {
            let pin = target.and_then(|t| zajil_core::tools::pinned(tool, t));
            ToolView {
                tool: name(tool).into(),
                installed: toolbox.is_installed(tool),
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
) -> Result<(), String> {
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
            .map_err(|e| e.to_string())?;
    }
    if installed {
        warm_up(&app, &stop).await?;
    }
    state.installing.lock().unwrap().take();
    Ok(())
}

/// [`VideoRunner::warm_up`] after an install: only a stop is said; a run
/// that could not start goes to the log (the first video will say why).
async fn warm_up(app: &crate::App, stop: &Stop) -> Result<(), String> {
    match runner(app).await.warm_up(stop).await {
        Err(zajil_core::video::VideoError::Stopped) => Err("stopped".into()),
        Err(e) => {
            eprintln!("zajil: the video tools' first run: {e}");
            Ok(())
        }
        Ok(()) => Ok(()),
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
    /// `drm`, `tools_missing`, `refused`, `tool`, `output`, `stopped` or
    /// `io`; for a failure of yt-dlp's whose words say what it comes to,
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
    let kind = match (f.has_video(), f.has_audio()) {
        (true, true) => "both",
        (true, false) => "video",
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
    let sent = runner(&app)
        .await
        .probe_sent(&url, media.as_deref(), cookies.as_deref(), &stop)
        .await?;
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
/// named after its title; they start in turn, at most [`MAX_RUNNING`] at
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
    }
}

/// Runs video download `id` of the list; `ask` asks its page first (a
/// resumed one: the formats of the last probe may be gone, or stale).
fn start<R: tauri::Runtime>(handle: tauri::AppHandle<R>, id: u64, ask: bool) {
    let state = handle.state::<VideoState>();
    let Some(row) = with_row(&state, id, |row| {
        row.status = StatusView::Active;
        row.clone()
    }) else {
        return;
    };
    let stop = Stop::new();
    state.downloads.lock().unwrap().insert(id, stop.clone());
    let known = state.formats.lock().unwrap().get(&row.url).cloned();
    tauri::async_runtime::spawn(async move {
        let runner = runner(&handle.state::<crate::App>()).await;
        let formats = match known.filter(|_| !ask) {
            Some(formats) => Ok(formats),
            None => match probe(
                handle.state::<VideoState>().inner(),
                &runner,
                &row.url,
                &stop,
            )
            .await
            {
                Ok(Probed::Video(video)) => Ok(video.formats),
                Ok(Probed::Playlist { .. }) => Err(VideoErrorView {
                    kind: "playlist".into(),
                    message: "the page is a playlist".into(),
                }),
                Err(e) => Err(e.into()),
            },
        };
        // The engine's best, once chosen, is the row's choice from now on.
        let chosen = formats.and_then(|formats| {
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
        let result = match chosen {
            Ok((formats, choice)) => {
                let request = Download {
                    url: &row.url,
                    choice: &choice,
                    formats: &formats,
                    cookies: cookies.as_deref(),
                    dest: std::path::Path::new(&row.folder),
                    name: &row.name,
                };
                let events = handle.clone();
                runner
                    .download(
                        &request,
                        &stop,
                        &mut |VideoEvent::Progress { done, total, speed }| {
                            progressed(&events, id, done, total, speed);
                        },
                    )
                    .await
                    .map_err(VideoErrorView::from)
            }
            Err(e) => Err(e),
        };
        let state = handle.state::<VideoState>();
        state.downloads.lock().unwrap().remove(&id);
        state.told.lock().unwrap().remove(&id);
        let lang = handle
            .state::<crate::App>()
            .settings
            .load()
            .map(|s| s.language)
            .unwrap_or_default();
        with_row(&state, id, |row| match &result {
            Ok(file) => {
                row.status = StatusView::Done;
                row.file = Some(file.display().to_string());
                row.size = std::fs::metadata(file).ok().map(|m| m.len()).or(row.size);
                row.done = row.size.unwrap_or(row.done);
            }
            Err(e) if e.kind == "stopped" => {
                row.status = StatusView::Paused;
                row.done = 0;
            }
            Err(e) => {
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

    /// After a restart, a video that ran is paused from zero (its run was
    /// swept); a file changed by hand never takes a queue's id.
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
        assert_eq!((back[0].status, back[0].done), (StatusView::Paused, 0));
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
    /// [`MAX_RUNNING`] run: the rest wait their turn, first come first.
    #[test]
    fn a_playlist_s_videos_wait_their_turn() {
        let mut rows: Vec<Row> = (1..=5)
            .map(|n| new_row(FIRST_ID + n, item("https://example.com/v", Some("v")), "/d"))
            .collect();
        let none = HashSet::new();
        assert_eq!(to_start(&rows, &none), [FIRST_ID + 1, FIRST_ID + 2]);
        let two: HashSet<u64> = [FIRST_ID + 1, FIRST_ID + 2].into();
        for row in &mut rows[..2] {
            row.status = StatusView::Active;
        }
        assert!(to_start(&rows, &two).is_empty());
        // One ended, one paused: the next waiting one, not the paused.
        rows[0].status = StatusView::Done;
        rows[2].status = StatusView::Paused;
        let one: HashSet<u64> = [FIRST_ID + 2].into();
        assert_eq!(to_start(&rows, &one), [FIRST_ID + 4]);
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
