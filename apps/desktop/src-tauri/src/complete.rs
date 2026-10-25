//! «Download complete» (M5.5, the design of #83, window 5): a window of its
//! own for each download that finishes, as in IDM, with its name, size,
//! folder and link, «Open», «Open folder» and «Close», and «Don't show this
//! window again». Drawn by the same page (`index.html?win=complete&id=…`).
//!
//! Whether it shows is the user's choice, kept by the page (as the table's
//! columns are) and told to Rust at each start and each change: it shows
//! until the page says otherwise. Never for a download from a private
//! window (review of #91): nothing of it stays but the file, so it is out
//! of the queues by the time it would show.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{Manager, State};
use ts_rs::TS;
use zajil_core::queue::{Item, QueueEvent, Status, shown_url};

use crate::App;

/// The complete windows' capability: Zajil's events and the yes-or-no
/// question before a program opens. Added when the app starts.
pub const CAPABILITY: &str = include_str!("../runtime-capabilities/complete.json");

/// Each window's label is this and the download's id.
pub const PREFIX: &str = "complete-";

/// The window's size, in logical pixels.
pub const SIZE: (f64, f64) = (560.0, 320.0);

/// Whether the window shows when a download finishes.
#[derive(Debug)]
pub struct Show(AtomicBool);

impl Default for Show {
    fn default() -> Self {
        Show(AtomicBool::new(true))
    }
}

/// What the window shows of a finished download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CompleteView {
    pub id: u64,
    pub name: String,
    /// Bytes; `null` when the file cannot be read now.
    #[ts(type = "number | null")]
    pub size: Option<u64>,
    /// The folder it is in.
    pub folder: String,
    /// The link, without its query, fragment, user name or password (a
    /// token is never shown).
    pub url: String,
}

/// The page's choice: the window shows (`true`) or not.
#[tauri::command]
pub fn set_show_complete(show: State<'_, Show>, on: bool) {
    show.0.store(on, Ordering::Relaxed);
}

/// What window `complete-<id>` shows.
#[tauri::command]
pub async fn complete_info(app: State<'_, App>, id: u64) -> Result<CompleteView, String> {
    let store = app.store.clone();
    tokio::task::spawn_blocking(move || {
        let queues = store.load().map_err(|e| e.to_string())?;
        let item = queues.item(id).ok_or_else(|| format!("no download {id}"))?;
        let size = item
            .file
            .as_deref()
            .and_then(|f| std::fs::metadata(f).ok())
            .filter(|m| m.is_file())
            .map(|m| m.len());
        view(item, size).ok_or_else(|| format!("download {id} has no finished file"))
    })
    .await
    .map_err(crate::blocking_err)?
}

/// The window's view of `item`, if it is done with a file.
fn view(item: &Item, size: Option<u64>) -> Option<CompleteView> {
    if item.status != Status::Done {
        return None;
    }
    let file = item.file.as_deref()?;
    Some(CompleteView {
        id: item.id,
        name: file.file_name()?.to_string_lossy().into_owned(),
        size,
        folder: file
            .parent()
            .unwrap_or(Path::new(""))
            .to_string_lossy()
            .into_owned(),
        url: shown_url(&item.url),
    })
}

/// Whether a finished `item` gets the window.
fn shows_for(item: &Item) -> bool {
    !item.private && item.status == Status::Done && item.file.is_some()
}

/// A download finished: its window opens, unless the user turned it off.
pub fn on_event(handle: &tauri::AppHandle, event: &QueueEvent) {
    let QueueEvent::Finished { item, .. } = event else {
        return;
    };
    if !handle.state::<Show>().0.load(Ordering::Relaxed) {
        return;
    }
    let (handle, id) = (handle.clone(), *item);
    tauri::async_runtime::spawn(async move {
        let (store, settings) = {
            let app = handle.state::<App>();
            (app.store.clone(), app.settings.clone())
        };
        let title = tokio::task::spawn_blocking(move || {
            let wanted = store
                .load()
                .ok()
                .and_then(|q| q.item(id).map(shows_for))
                .unwrap_or(false);
            wanted.then(|| title(settings.load().map(|s| s.language).unwrap_or_default()))
        })
        .await;
        let Ok(Some(title)) = title else {
            return;
        };
        let label = format!("{PREFIX}{id}");
        let page = format!("complete&id={id}");
        if let Err(err) = crate::windows::open_page(&handle, &label, &page, &title, SIZE) {
            eprintln!("zajil: the «download complete» window: {err}");
        }
    });
}

/// The window's title in the user's language.
fn title(lang: zajil_core::settings::Language) -> String {
    let lang = match lang {
        zajil_core::settings::Language::Ar => zajil_i18n::Lang::Ar,
        zajil_core::settings::Language::En => zajil_i18n::Lang::En,
    };
    zajil_i18n::Localizer::new(lang)
        .text("complete-title", &[])
        .unwrap_or_else(|_| "Download complete".to_owned())
}

/// The complete windows close with the main one, as the add windows do.
pub fn close_with_main<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    let Some(main) = handle.get_webview_window("main") else {
        return;
    };
    let app = handle.clone();
    main.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            for (label, window) in app.webview_windows() {
                if label.starts_with(PREFIX) {
                    let _ = window.close();
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use zajil_core::queue::{DEFAULT_QUEUE, QueuesFile};

    fn done(url: &str) -> Item {
        let mut q = QueuesFile::empty();
        let dir = std::env::temp_dir();
        let id = q.add(DEFAULT_QUEUE, url, &dir).unwrap();
        let item = q.item_mut(id).unwrap();
        item.status = Status::Done;
        item.file = Some(dir.join("report.pdf"));
        item.clone()
    }

    #[test]
    fn the_window_shows_the_file_and_a_link_without_its_token() {
        let item = done("https://files.test/get/report.pdf?sig=SECRET#part");
        let shown = view(&item, Some(42)).unwrap();
        assert_eq!(shown.name, "report.pdf");
        assert_eq!(shown.size, Some(42));
        let file = item.file.as_deref().unwrap();
        assert_eq!(shown.folder, file.parent().unwrap().to_string_lossy());
        assert_eq!(shown.url, "https://files.test/get/report.pdf");
        assert!(!format!("{shown:?}").contains("SECRET"));
    }

    #[test]
    fn only_a_finished_download_of_a_normal_window_gets_it() {
        let item = done("https://files.test/a.zip");
        assert!(shows_for(&item));
        let private = Item {
            private: true,
            ..item.clone()
        };
        assert!(!shows_for(&private));
        let failed = Item {
            status: Status::Failed,
            ..item.clone()
        };
        assert!(!shows_for(&failed) && view(&failed, None).is_none());
        let no_file = Item { file: None, ..item };
        assert!(!shows_for(&no_file));
    }

    #[test]
    fn it_shows_until_the_page_says_otherwise() {
        assert!(Show::default().0.load(Ordering::Relaxed));
    }

    /// The complete windows may listen and ask before a program opens, and
    /// nothing more.
    #[test]
    fn the_complete_windows_have_only_their_own_permissions() {
        let capability: serde_json::Value = serde_json::from_str(CAPABILITY).unwrap();
        assert_eq!(
            capability["windows"],
            serde_json::json!([format!("{PREFIX}*")])
        );
        assert_eq!(
            capability["permissions"],
            serde_json::json!(["core:event:default", "dialog:allow-message"])
        );
    }
}
