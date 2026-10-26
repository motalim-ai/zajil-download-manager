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

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};
use ts_rs::TS;
use zajil_core::Stop;
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
    /// Its license, as ADR-0009 says.
    pub license: String,
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
                license: match tool {
                    Tool::YtDlp => "GPLv3+",
                    Tool::QuickJs => "MIT",
                    Tool::Ffmpeg | Tool::Ffprobe => "LGPL 2.1+",
                }
                .into(),
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

/// Downloads a probed video into `dest` as `name`; its id at once, then
/// `video-progress`, and `video-finished` or `video-failed`.
#[tauri::command]
pub fn video_download<R: tauri::Runtime>(
    state: State<'_, VideoState>,
    url: String,
    choice: ChoiceView,
    dest: String,
    name: String,
    handle: tauri::AppHandle<R>,
) -> Result<u64, String> {
    let id = state.next.fetch_add(1, Ordering::SeqCst) + 1;
    let stop = Stop::new();
    state.downloads.lock().unwrap().insert(id, stop.clone());
    let formats = state
        .formats
        .lock()
        .unwrap()
        .get(&url)
        .cloned()
        .unwrap_or_default();
    let cookies = state.cookies.lock().unwrap().get(&url).cloned();
    tauri::async_runtime::spawn(async move {
        let runner = runner(&handle.state::<crate::App>()).await;
        let choice = Choice::from(choice);
        let request = Download {
            url: &url,
            choice: &choice,
            formats: &formats,
            cookies: cookies.as_deref(),
            dest: std::path::Path::new(&dest),
            name: &name,
        };
        let events = handle.clone();
        let result = runner
            .download(
                &request,
                &stop,
                &mut |VideoEvent::Progress { done, total, speed }| {
                    let _ = events.emit(
                        "video-progress",
                        Progress {
                            id,
                            done,
                            total,
                            speed,
                        },
                    );
                },
            )
            .await;
        handle
            .state::<VideoState>()
            .downloads
            .lock()
            .unwrap()
            .remove(&id);
        match result {
            Ok(file) => {
                let _ = handle.emit(
                    "video-finished",
                    serde_json::json!({ "id": id, "file": file.display().to_string() }),
                );
            }
            Err(e) => {
                let _ = handle.emit(
                    "video-failed",
                    serde_json::json!({ "id": id, "error": VideoErrorView::from(e) }),
                );
            }
        }
    });
    Ok(id)
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
