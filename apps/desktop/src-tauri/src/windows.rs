//! The add windows (M5.5, the design of #83, windows 2 and 3): «Add a
//! link» and «Download file info» are windows of their own, as in IDM, not
//! dialogs inside the main window. The page draws them too
//! (`index.html?win=…`); what each one starts with waits here, so no link
//! goes into a window's address. They have their own capability (`add`):
//! listening to Zajil's events and the system's folder picker, nothing
//! else.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use ts_rs::TS;

use crate::App;

/// The add windows' capability: listening to events and the folder
/// picker. Added when the app starts, for those two windows only.
pub const CAPABILITY: &str = include_str!("../runtime-capabilities/add.json");

/// The progress windows' capability: listening to events, and their own
/// title (the percentage and the name) and size (with or without the
/// details); the system's question before a program opens (#84).
pub const PROGRESS_CAPABILITY: &str = include_str!("../runtime-capabilities/progress.json");

pub const LINK: &str = "add-link";
pub const INFO: &str = "add-info";

/// The «Add a link» window's size, in logical pixels.
pub const LINK_SIZE: (f64, f64) = (560.0, 230.0);
/// The «Download file info» window's.
pub const INFO_SIZE: (f64, f64) = (680.0, 500.0);

/// A progress window's size with its details shown (design window 4).
pub const PROGRESS_SIZE: (f64, f64) = (600.0, 640.0);

/// The progress window of download `id`.
pub fn progress_label(id: u64) -> String {
    format!("progress-{id}")
}

/// What «Add a link» starts with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PendingLink {
    pub url: String,
    /// It came from the clipboard: the window says so.
    pub from_clipboard: bool,
}

/// What each add window starts with.
#[derive(Debug, Default)]
pub struct Pending {
    link: Option<PendingLink>,
    info: Option<String>,
}

pub type Shared = Mutex<Pending>;

/// Opens window `label` (`win` in the page's address), or brings it to the
/// front if it is open.
fn open<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    label: &str,
    title: &str,
    (width, height): (f64, f64),
) -> tauri::Result<()> {
    if let Some(window) = handle.get_webview_window(label) {
        let _ = window.unminimize();
        window.show()?;
        return window.set_focus();
    }
    let window = WebviewWindowBuilder::new(
        handle,
        label,
        WebviewUrl::App(format!("index.html?win={label}").into()),
    )
    .title(title)
    .inner_size(width, height)
    .min_inner_size(
        width,
        height.min(if label == INFO || label == LINK {
            height
        } else {
            320.0
        }),
    )
    .resizable(true)
    .maximizable(false)
    .minimizable(false)
    .center()
    .focused(true)
    .build()?;
    // The info window closed: a check of the server still running is given
    // up (as the dialog's cancel did).
    if label == INFO {
        let app = handle.clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::Destroyed = event
                && let Some(stop) = app.state::<App>().probing.lock().unwrap().take()
            {
                stop.request();
            }
        });
    }
    Ok(())
}

/// «Add a link»: with `url` (a copied link, or none), titled `title` in
/// the page's language. An open one takes the new link.
#[tauri::command]
pub fn open_add_link<R: tauri::Runtime>(
    pending: State<'_, Shared>,
    url: Option<String>,
    from_clipboard: bool,
    title: String,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    if crate::in_scene(&handle) {
        return Ok(());
    }
    let link = PendingLink {
        url: url.unwrap_or_default(),
        from_clipboard,
    };
    pending.lock().unwrap().link = Some(link.clone());
    let _ = handle.emit_to(LINK, "pending-link", link);
    open(&handle, LINK, &title, LINK_SIZE).map_err(|e| e.to_string())
}

/// The link «Add a link» starts with.
#[tauri::command]
pub fn pending_link(pending: State<'_, Shared>) -> Option<PendingLink> {
    pending.lock().unwrap().link.clone()
}

/// «Download file info» for `url`, titled `title`; «Add a link» closes.
/// One info window at a time: while one is open, it comes to the front and
/// [`INFO_OPEN`] is the answer.
#[tauri::command]
pub fn open_add_info<R: tauri::Runtime>(
    pending: State<'_, Shared>,
    url: String,
    title: String,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    if crate::in_scene(&handle) {
        return Ok(());
    }
    // One at a time: another link would be lost behind the open one
    // (review of #104), so the link window stays and says so.
    if let Some(info) = handle.get_webview_window(INFO) {
        let _ = info.set_focus();
        return Err(INFO_OPEN.into());
    }
    if let Some(link) = handle.get_webview_window(LINK) {
        let _ = link.close();
    }
    pending.lock().unwrap().info = Some(url);
    open(&handle, INFO, &title, INFO_SIZE).map_err(|e| e.to_string())
}

/// What [`open_add_info`] answers while «Download file info» is open: the
/// page says it in the user's language.
pub const INFO_OPEN: &str = "info-open";

/// The other windows close when the main one does: without it the program
/// would go on with no main window to come back to (review of #104).
pub fn close_with_main<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    let Some(main) = handle.get_webview_window("main") else {
        return;
    };
    let app = handle.clone();
    main.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            for (label, window) in app.webview_windows() {
                if label != "main" {
                    let _ = window.close();
                }
            }
        }
    });
}

/// The link «Download file info» is for.
#[tauri::command]
pub fn pending_info(pending: State<'_, Shared>) -> Option<String> {
    pending.lock().unwrap().info.clone()
}

/// Closes the window that asks.
#[tauri::command]
pub fn close_window<R: tauri::Runtime>(window: tauri::WebviewWindow<R>) -> Result<(), String> {
    window.close().map_err(|e| e.to_string())
}

/// The progress window of download `id` (design window 4), one per
/// download as in IDM, titled `title`; an open one comes to the front. The
/// page reads the download's id from the window's label.
#[tauri::command]
pub fn open_progress<R: tauri::Runtime>(
    id: u64,
    title: String,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    if crate::in_scene(&handle) {
        return Ok(());
    }
    open(&handle, &progress_label(id), &title, PROGRESS_SIZE).map_err(|e| e.to_string())
}

/// Shows the main window on `view` (the categories editor, from «New
/// category»).
#[tauri::command]
pub fn show_main_view<R: tauri::Runtime>(view: String, handle: tauri::AppHandle<R>) {
    crate::tray::show_window(&handle);
    let _ = handle.emit_to("main", "main-view", view);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_starts_with_nothing_waiting() {
        let pending = Pending::default();
        assert!(pending.link.is_none() && pending.info.is_none());
        assert_eq!(LINK, "add-link");
        assert_eq!(INFO, "add-info");
        assert_eq!(progress_label(7), "progress-7");
    }

    /// The progress windows may listen, set their own title and size, and
    /// ask before a program opens; nothing more.
    #[test]
    fn the_progress_windows_have_only_their_own_permissions() {
        let capability: serde_json::Value = serde_json::from_str(PROGRESS_CAPABILITY).unwrap();
        assert_eq!(capability["windows"], serde_json::json!(["progress-*"]));
        assert_eq!(
            capability["permissions"],
            serde_json::json!([
                "core:event:default",
                "core:window:allow-set-title",
                "core:window:allow-set-size",
                "dialog:allow-message"
            ])
        );
    }

    /// The add windows may listen and pick a folder, and nothing more; the
    /// main window's permissions are not theirs.
    #[test]
    fn the_add_windows_have_only_their_own_permissions() {
        let capability: serde_json::Value = serde_json::from_str(CAPABILITY).unwrap();
        assert_eq!(capability["windows"], serde_json::json!([LINK, INFO]));
        assert_eq!(
            capability["permissions"],
            serde_json::json!(["core:event:default", "dialog:allow-open"])
        );
    }
}
