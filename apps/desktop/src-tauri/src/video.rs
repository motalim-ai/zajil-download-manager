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

use std::collections::HashMap;
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
    Choice, Download, Format, Probed, Runs, VideoError, VideoEvent, VideoRunner,
};

const TOOLS: [Tool; 4] = Tool::ALL;

/// What the app keeps for video downloads.
#[derive(Default)]
pub struct VideoState {
    /// A page's cookies, by link, as the extension sent them.
    cookies: Mutex<HashMap<String, String>>,
    /// The formats the last probe of each link found (a download names
    /// them).
    formats: Mutex<HashMap<String, Vec<Format>>>,
    /// The link «Video quality» opens with, from the extension.
    pending: Mutex<Option<String>>,
    installing: Mutex<Option<Stop>>,
    probing: Mutex<Option<Stop>>,
    downloads: Mutex<HashMap<u64, Stop>>,
    next: AtomicU64,
    /// The video downloads of the list (#149): kept in `videos.json`.
    rows: Mutex<Vec<Row>>,
    /// When each running one last told the list (at most four times a
    /// second, as the queues' downloads).
    told: Mutex<HashMap<u64, Instant>>,
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

/// Writes the list's video downloads whole (a new file, then renamed).
fn save_rows(app: &crate::App, state: &VideoState) {
    let rows = state.rows.lock().unwrap().clone();
    let path = rows_file(app);
    let saved = serde_json::to_vec_pretty(&rows)
        .map_err(std::io::Error::other)
        .and_then(|bytes| {
            let new = path.with_extension("json.new");
            std::fs::write(&new, bytes)?;
            std::fs::rename(&new, &path)
        });
    if let Err(e) = saved {
        eprintln!("zajil: {ROWS_FILE} could not be saved: {e}");
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

/// «Pause»: a running video stops (it starts over when resumed).
pub fn pause(state: &VideoState, ids: &[u64]) {
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
        let again = with_row(&state, id, |row| {
            let again = matches!(row.status, StatusView::Paused | StatusView::Failed);
            if again {
                row.status = StatusView::Waiting;
                row.error = None;
                row.done = 0;
                row.size = None;
            }
            again
        });
        if again == Some(true) {
            start(handle.clone(), id, true);
        }
    }
    save_rows(&handle.state::<crate::App>(), &state);
    let _ = handle.emit("downloads-changed", ());
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

/// A video from the extension: its cookies kept here, and «Video quality»
/// opened for its link.
pub fn receive<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    url: String,
    cookies: Option<String>,
) {
    let state = handle.state::<VideoState>();
    if let Some(cookies) = cookies {
        state.cookies.lock().unwrap().insert(url.clone(), cookies);
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
pub fn open_video<R: tauri::Runtime>(
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

#[tauri::command]
pub fn video_tools_status(app: State<'_, crate::App>) -> Vec<ToolView> {
    let toolbox = Toolbox::new(folders(&app).0);
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
#[tauri::command]
pub async fn video_tools_install<R: tauri::Runtime>(
    app: State<'_, crate::App>,
    state: State<'_, VideoState>,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    let stop = Stop::new();
    *state.installing.lock().unwrap() = Some(stop.clone());
    let toolbox = Toolbox::new(folders(&app).0);
    for tool in TOOLS {
        if toolbox.is_installed(tool) {
            continue;
        }
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
    state.installing.lock().unwrap().take();
    Ok(())
}

#[tauri::command]
pub fn video_tools_cancel(state: State<'_, VideoState>) {
    if let Some(stop) = state.installing.lock().unwrap().take() {
        stop.request();
    }
}

/// «Delete the video tools» (the settings).
#[tauri::command]
pub fn video_tools_remove(app: State<'_, crate::App>) -> Result<(), String> {
    Toolbox::new(folders(&app).0)
        .remove_all()
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
    Format { id: String },
    Merge { video: String, audio: String },
    AudioOnly,
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

impl From<ChoiceView> for Choice {
    fn from(c: ChoiceView) -> Self {
        match c {
            ChoiceView::Format { id } => Choice::Format(id),
            ChoiceView::Merge { video, audio } => Choice::Merge { video, audio },
            ChoiceView::AudioOnly => Choice::AudioOnly,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct EntryView {
    pub url: Option<String>,
    pub title: Option<String>,
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
    /// `io`.
    pub kind: String,
    pub message: String,
}

impl From<VideoError> for VideoErrorView {
    fn from(e: VideoError) -> Self {
        let kind = match &e {
            VideoError::Drm => "drm",
            VideoError::Tools(_) => "tools_missing",
            VideoError::Refused(_) => "refused",
            VideoError::Tool(_) => "tool",
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
    let probed = runner(&app)
        .await
        .probe(&url, cookies.as_deref(), &stop)
        .await?;
    state.probing.lock().unwrap().take();
    Ok(match probed {
        Probed::Video(video) => {
            let best = video.best_choice().map(ChoiceView::from);
            let formats = video.formats.iter().map(format_view).collect();
            state
                .formats
                .lock()
                .unwrap()
                .insert(url, video.formats.clone());
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
                    url: e.url,
                    title: e.title,
                })
                .collect(),
        },
    })
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
    });
    save_rows(&app, &state);
    let _ = handle.emit("downloads-changed", ());
    start(handle, id, false);
    Ok(id)
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
    let cookies = state.cookies.lock().unwrap().get(&row.url).cloned();
    let known = state.formats.lock().unwrap().get(&row.url).cloned();
    tauri::async_runtime::spawn(async move {
        let runner = runner(&handle.state::<crate::App>()).await;
        let formats = match known.filter(|_| !ask) {
            Some(formats) => Ok(formats),
            None => match runner.probe(&row.url, cookies.as_deref(), &stop).await {
                Ok(Probed::Video(video)) => Ok(video.formats),
                Ok(Probed::Playlist { .. }) => {
                    Err(VideoError::Refused("the page is a playlist now".into()))
                }
                Err(e) => Err(e),
            },
        };
        let choice = Choice::from(row.choice.clone());
        let result = match formats {
            Ok(formats) => {
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
            }
            Err(e) => Err(e),
        };
        let state = handle.state::<VideoState>();
        state.downloads.lock().unwrap().remove(&id);
        state.told.lock().unwrap().remove(&id);
        let result = result.map_err(VideoErrorView::from);
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
                row.error = Some(e.message.clone());
            }
        });
        save_rows(&handle.state::<crate::App>(), &state);
        let _ = handle.emit("downloads-changed", ());
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
            assert_eq!(Choice::from(ChoiceView::from(choice.clone())), choice);
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
}
