//! The video tools (yt-dlp and FFmpeg): their status, install, update
//! and removal.

use serde::Serialize;
use tauri::{Emitter, Manager, State};
use ts_rs::TS;
use zajil_core::Stop;

use super::views::VideoErrorView;
use super::{VideoState, folders, runner};
use zajil_core::tools::{Tool, Toolbox};
use zajil_core::video::VideoError;

const TOOLS: [Tool; 4] = Tool::ALL;

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
            let pin = target.and_then(|_| toolbox.pinned(tool));
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
}
