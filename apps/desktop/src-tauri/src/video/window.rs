//! «Video quality»: a video from the browser, the window and its
//! commands.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use serde::Serialize;
use tauri::{Emitter, Manager, State};
use ts_rs::TS;

use super::choice::same_itags;
use super::queue::{known_queue, pause, pump, resume};
use super::rows::{remove_leftovers, save_rows, with_row};
use super::views::ChoiceView;
use super::{FIRST_ID, MoveTo, Row, VideoState};
use crate::view::StatusView;

/// «Video quality»'s window, and its capability: events, its own title,
/// and the folder picker.
pub const LABEL: &str = "video";
pub const SIZE: (f64, f64) = (640.0, 520.0);
pub const CAPABILITY: &str = include_str!("../../runtime-capabilities/video.json");

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
            folder: crate::downloads::download_dir(&app.settings, &app.system_download_dir)
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
            queue: None,
            agents: Vec::new(),
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
    queue: Option<String>,
) -> Result<(), String> {
    let state = handle.state::<VideoState>();
    state.started.lock().unwrap().take_if(|s| *s == id);
    let queue = known_queue(&handle.state::<crate::App>(), queue);
    with_row(&state, id, |row| row.queue = queue);
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
    queue: Option<String>,
) -> Result<(), String> {
    video_keep(handle.clone(), id, folder, name, queue).await?;
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
pub(super) fn discard<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, id: u64) {
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
    crate::downloads::download_dir(&app.settings, &app.system_download_dir)
        .display()
        .to_string()
}

/// The link the extension sent, once (the page opens «Video quality»).
#[tauri::command]
pub fn video_pending(state: State<'_, VideoState>) -> Option<String> {
    state.pending.lock().unwrap().take()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zajil_core::Stop;

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
}
