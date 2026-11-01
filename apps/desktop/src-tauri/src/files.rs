//! A download's files: opening them or their folder, and the queues
//! exported to or imported from a file the user chose.

use std::path::Path;

use tauri::{Emitter, Manager, State};

use crate::downloads::download_dir;
#[cfg(feature = "ui-test")]
use crate::ui_test;
use crate::view::ImportView;
use crate::{App, blocking_err, in_scene, video};

/// Opens a finished download (`folder` false) with the system's program
/// for it, or shows it in its folder (`folder` true; a download with no
/// file yet shows its folder, selected in the one above, never opened: an
/// imported folder could be a program, as a macOS `.app` is). Only a
/// download's own paths: the page names the download, never a path. A file
/// that can run code (#84) opens only once the user is `sure`: `false`
/// comes back, and the page asks.
#[tauri::command]
pub(crate) async fn open_download<R: tauri::Runtime>(
    app: State<'_, App>,
    id: u64,
    folder: bool,
    sure: bool,
    handle: tauri::AppHandle<R>,
) -> Result<bool, String> {
    use tauri_plugin_opener::OpenerExt;
    if in_scene(&handle) {
        return Ok(true);
    }
    let (file, dir) = if video::is_video(id) {
        video::file_of(&handle.state::<video::VideoState>(), id)
            .ok_or_else(|| format!("no download {id}"))?
    } else {
        let store = app.store.clone();
        let queues = tokio::task::spawn_blocking(move || store.load())
            .await
            .map_err(blocking_err)?
            .map_err(|e| e.to_string())?;
        let item = queues.item(id).ok_or_else(|| format!("no download {id}"))?;
        (item.file.clone().filter(|f| f.is_file()), item.dir.clone())
    };
    // The end-to-end test: what would be opened is written down (no file
    // manager on the CI machines' screens).
    #[cfg(feature = "ui-test")]
    if handle.state::<ui_test::UiTest>().is_e2e() {
        let shown = file.as_deref().unwrap_or(&dir);
        handle.state::<ui_test::UiTest>().step(
            "open",
            serde_json::json!({ "folder": folder, "path": shown }),
        )?;
        return Ok(true);
    }
    let opener = handle.opener();
    match (folder, file) {
        (false, Some(file)) if !sure && asks_before_opening(&file) => Ok(false),
        (false, Some(file)) => opener
            .open_path(file.to_string_lossy(), None::<&str>)
            .map(|()| true)
            .map_err(|e| e.to_string()),
        (false, None) => Err(OPEN_NO_FILE.into()),
        (true, Some(file)) => opener
            .reveal_item_in_dir(&file)
            .map(|()| true)
            .map_err(|e| e.to_string()),
        (true, None) if dir.is_dir() => opener
            .reveal_item_in_dir(&dir)
            .map(|()| true)
            .map_err(|e| e.to_string()),
        (true, None) => Err(OPEN_NO_FOLDER.into()),
    }
}

/// What `open_download` says when the finished file, or the folder, is not
/// there any more (moved or deleted): codes the page says in the user's
/// language (#98: it showed «download N has no finished file»).
pub const OPEN_NO_FILE: &str = "no-file";
pub const OPEN_NO_FOLDER: &str = "no-folder";

/// A file that can run code (#84): opening it runs it.
fn asks_before_opening(file: &std::path::Path) -> bool {
    file.file_name()
        .is_some_and(|n| zajil_core::category::runs_code(&n.to_string_lossy()))
}

/// Writes the queues to a new file the user chose (never over a file).
#[tauri::command]
pub(crate) async fn export_queues(app: State<'_, App>, path: String) -> Result<(), String> {
    let store = app.store.clone();
    tokio::task::spawn_blocking(move || {
        let queues = store.load().map_err(blocking_err)?;
        zajil_core::queue::write_export(Path::new(&path), &queues.export()).map_err(blocking_err)
    })
    .await
    .map_err(blocking_err)?
}

/// Imports a file the user chose, checked in full before anything changes.
/// Its folders are used only if the user asked; otherwise the downloads go
/// to the download folder, by category.
#[tauri::command]
pub(crate) async fn import_queues<R: tauri::Runtime>(
    app: State<'_, App>,
    path: String,
    keep_folders: bool,
    handle: tauri::AppHandle<R>,
) -> Result<ImportView, String> {
    if in_scene(&handle) {
        return Err("a ui-test scene imports nothing".into());
    }
    let (store, settings, system) = (
        app.store.clone(),
        app.settings.clone(),
        app.system_download_dir.clone(),
    );
    let done = tokio::task::spawn_blocking(move || {
        let export = zajil_core::queue::read_export(Path::new(&path)).map_err(blocking_err)?;
        let folders = if keep_folders {
            zajil_core::queue::Folders::Keep
        } else {
            zajil_core::queue::Folders::Into(download_dir(&settings, &system))
        };
        store
            .update(|queues| queues.import(&export, &folders))
            .map(|summary| ImportView::from(&summary))
            .map_err(blocking_err)
    })
    .await
    .map_err(blocking_err)??;
    let _ = handle.emit("downloads-changed", ());
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_runs_code_is_not_opened_without_asking() {
        use std::path::Path;
        assert!(asks_before_opening(Path::new("/d/setup.EXE")));
        assert!(asks_before_opening(Path::new("/d/run.sh")));
        assert!(!asks_before_opening(Path::new("/d/film.mkv")));
        assert!(!asks_before_opening(Path::new("/d/setup.exe.zip")));
    }
}
