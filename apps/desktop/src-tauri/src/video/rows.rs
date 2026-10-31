//! The video downloads of the list, kept in `videos.json`: their rows,
//! how the list shows them, and their removal.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use tauri::{Emitter, Manager};

use super::queue::pause;
use super::views::ChoiceView;
use super::window::discard;
use super::{FIRST_ID, ROWS_FILE, Row, VideoState, folders, is_video};
use crate::view::{DetailsView, DownloadView, StatusView};
use zajil_core::tools::Toolbox;
use zajil_core::video::Runs;

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
pub(super) fn save_rows(app: &crate::App, state: &VideoState) {
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

pub(super) fn with_row<T>(state: &VideoState, id: u64, f: impl FnOnce(&mut Row) -> T) -> Option<T> {
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
        queue: row.queue.clone().unwrap_or_default(),
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
    // Zajil's own way (YouTube) continues from its parts, yt-dlp from the
    // folder kept for the video: there once it began, its file once it
    // finished.
    let folder = std::path::Path::new(&row.folder);
    let resumable = row.status == StatusView::Done
        || !zajil_core::video::part_files(folder, &row.name).is_empty()
        || zajil_core::video::kept_folder(folder, &key_of(row.id)).is_dir();
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

/// What a cancelled download wrote: its parts, and its file if finished.
/// The video's own key, for the folder yt-dlp keeps for it.
pub(super) fn key_of(id: u64) -> String {
    format!("{id:x}")
}

pub(super) fn remove_leftovers(row: &Row) {
    let folder = std::path::Path::new(&row.folder);
    zajil_core::video::remove_kept(&zajil_core::video::kept_folder(folder, &key_of(row.id)));
    for part in zajil_core::video::part_files(folder, &row.name) {
        let _ = zajil_platform::fs::remove_file_retrying(&part);
    }
    if let Some(file) = &row.file {
        let _ = zajil_platform::fs::remove_file_retrying(std::path::Path::new(file));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::download::{NewVideo, new_row};
    use crate::video::tests::row;

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
}
