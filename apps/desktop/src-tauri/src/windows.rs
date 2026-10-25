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

pub const LINK: &str = "add-link";
pub const INFO: &str = "add-info";

/// The «Add a link» window's size, in logical pixels.
pub const LINK_SIZE: (f64, f64) = (560.0, 230.0);
/// The «Download file info» window's.
pub const INFO_SIZE: (f64, f64) = (680.0, 440.0);

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
    .min_inner_size(width, height)
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
/// One info window at a time: an open one keeps its link and comes to the
/// front.
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
    if let Some(link) = handle.get_webview_window(LINK) {
        let _ = link.close();
    }
    if handle.get_webview_window(INFO).is_none() {
        pending.lock().unwrap().info = Some(url);
    }
    open(&handle, INFO, &title, INFO_SIZE).map_err(|e| e.to_string())
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
