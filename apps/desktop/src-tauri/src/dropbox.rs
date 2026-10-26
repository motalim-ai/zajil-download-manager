//! The drop box (M5.5, an IDM touch): a small window, always on top of the
//! others, with Zajil's mark; a link (or text with a link) dropped on it
//! opens «Download file info». Off at first; shown or hidden from the
//! settings and the tray, hidden by a right-click on it; where the user
//! leaves it is kept (`window.json`, with the window preferences).
//!
//! Its page takes the drop itself: the window's own drop handling (files
//! from the system) is off, so the browser's drag and drop gives the page
//! the link. Its capability: events, and dragging the window.

use serde::{Deserialize, Serialize};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

use crate::closing::Closing;

pub const LABEL: &str = "drop";
/// Its size, in logical pixels.
pub const SIZE: f64 = 72.0;
/// Its capability: listening to Zajil's events and dragging itself.
pub const CAPABILITY: &str = include_str!("../runtime-capabilities/drop.json");

/// Where the drop box was left, in physical pixels on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub x: i32,
    pub y: i32,
}

/// Shows the drop box (where it was left, else near the top corner of the
/// screen), or hides it.
pub fn show<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, shown: bool) -> tauri::Result<()> {
    if !shown {
        if let Some(window) = handle.get_webview_window(LABEL) {
            window.close()?;
        }
        return Ok(());
    }
    if let Some(window) = handle.get_webview_window(LABEL) {
        return window.show();
    }
    let mut builder =
        WebviewWindowBuilder::new(handle, LABEL, WebviewUrl::App("index.html?win=drop".into()))
            .title("Zajil")
            .inner_size(SIZE, SIZE)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .shadow(true)
            .disable_drag_drop_handler();
    if let Some(place) = handle.state::<Closing>().prefs().drop_place {
        builder = builder.position(0.0, 0.0);
        let window = builder.build()?;
        window.set_position(tauri::PhysicalPosition::new(place.x, place.y))?;
        keep_place(handle, &window);
        return Ok(());
    }
    let window = builder.build()?;
    if let Some(monitor) = window.current_monitor()? {
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
        let side = (SIZE * scale) as i32;
        window.set_position(tauri::PhysicalPosition::new(
            area.position.x + area.size.width as i32 - side - (40.0 * scale) as i32,
            area.position.y + (120.0 * scale) as i32,
        ))?;
    }
    keep_place(handle, &window);
    Ok(())
}

/// Keeps where the user moves it.
fn keep_place<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, window: &tauri::WebviewWindow<R>) {
    let app = handle.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Moved(at) = event {
            let place = Place { x: at.x, y: at.y };
            let _ = app
                .state::<Closing>()
                .change(|p| p.drop_place = Some(place));
        }
    });
}

/// Whether the drop box is shown (the settings).
#[tauri::command]
pub fn drop_box_shown(closing: tauri::State<'_, Closing>) -> bool {
    closing.prefs().drop_box
}

/// Shows or hides the drop box, and keeps the choice.
#[tauri::command]
pub async fn set_drop_box<R: tauri::Runtime>(
    closing: tauri::State<'_, Closing>,
    shown: bool,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    if crate::in_scene(&handle) {
        return Ok(());
    }
    closing
        .change(|p| p.drop_box = shown)
        .map_err(|e| e.to_string())?;
    show(&handle, shown).map_err(|e| e.to_string())
}

/// The tray's «Show the drop box»: shown if hidden, hidden if shown.
pub fn toggle<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    let shown = !handle.state::<Closing>().prefs().drop_box;
    let _ = handle.state::<Closing>().change(|p| p.drop_box = shown);
    let _ = show(handle, shown);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The drop box may listen, drag itself, and show its right-click menu
    /// (M7: «Batch download», hiding it); nothing more.
    #[test]
    fn the_drop_box_has_only_its_own_permissions() {
        let capability: serde_json::Value = serde_json::from_str(CAPABILITY).unwrap();
        assert_eq!(capability["windows"], serde_json::json!([LABEL]));
        assert_eq!(
            capability["permissions"],
            serde_json::json!([
                "core:event:default",
                "core:window:allow-start-dragging",
                "core:menu:default"
            ])
        );
    }
}
