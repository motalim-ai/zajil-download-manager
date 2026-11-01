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
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tauri::Manager;
use zajil_core::Stop;

use crate::view::StatusView;
use probe::Aheads;
use queue::pump;
use zajil_core::tools::{Tool, Toolbox};
use zajil_core::video::{Format, Runs, VideoRunner};

mod choice;
pub(crate) mod download;
pub(crate) mod probe;
pub(crate) mod queue;
mod rows;
pub(crate) mod tools;
mod views;
pub(crate) mod window;

pub use probe::read_ahead;
pub use queue::{QUEUE_VIDEO_DONE, pause, queue_videos_done, resume, set_queue, watch_queues};
pub use rows::{
    clean_leftovers, details, discard_private, file_of, load_rows, remove, remove_with_files,
    url_of, views,
};
pub use tools::{update_tools, video_tools_status};
pub use views::ChoiceView;
pub use window::{CAPABILITY, Chosen, asked, receive};

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
    /// Downloads stopped because their queue's window closed: once
    /// stopped, they wait for it to open again.
    requeue: Mutex<HashSet<u64>>,
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

/// One video download of the list. A stopped one continues from what it
/// saved (Zajil's own parts, or yt-dlp's kept folder); «resume» may ask
/// the page again for new links.
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
    /// The queue it runs in (owner's request, 2026-10-30): it starts when
    /// its queue may run, within its downloads at once, and the queue's
    /// «after it ends» waits for it. None: the videos' own list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queue: Option<String>,
    /// The program each part's link was made for (format id, `User-Agent`),
    /// for a resume with the saved links: YouTube refuses some to another.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    agents: Vec<(String, String)>,
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

#[cfg(test)]
mod tests {
    use super::download::NewVideo;
    use super::*;

    /// Videos' ids never meet the queues', and stay exact in JavaScript.
    #[test]
    fn video_ids_are_their_own() {
        assert!(!is_video(1) && !is_video(FIRST_ID - 1) && is_video(FIRST_ID + 1));
        const { assert!(FIRST_ID * 2 <= 1 << 53) };
        let (videos, queues) = split(vec![3, FIRST_ID + 2, 7]);
        assert_eq!((videos, queues), (vec![FIRST_ID + 2], vec![3, 7]));
    }

    pub(super) fn row(id: u64, status: StatusView) -> Row {
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
            queue: None,
            agents: Vec::new(),
        }
    }

    pub(super) fn item(url: &str, title: Option<&str>) -> NewVideo {
        NewVideo {
            url: url.into(),
            title: title.map(Into::into),
        }
    }
}
