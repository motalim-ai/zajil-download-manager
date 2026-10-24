//! The icon in the system tray (M4): a click shows the window (Windows and
//! macOS; on Linux Tauri reports no clicks, so there it is the menu only);
//! its menu shows the window or quits, the same way as closing it (the
//! downloads record their progress and stop first). On macOS the icon is the mark
//! as a template image (icons/tray-template.png, made by
//! scripts/tray-icon.mjs), coloured by the system to suit the menu bar;
//! on Windows and Linux it is the app icon. The menu's texts follow the
//! interface's language ([`set_texts`], from the page).

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

const ID: &str = "zajil";

fn menu<R: Runtime, M: Manager<R>>(app: &M, show: &str, quit: &str) -> tauri::Result<Menu<R>> {
    let show = MenuItem::with_id(app, "show", show, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", quit, true, None::<&str>)?;
    Menu::with_items(app, &[&show, &quit])
}

/// Puts the icon in the tray. Arabic until the page gives the texts of
/// the chosen language.
pub fn build<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    let tray = TrayIconBuilder::with_id(ID)
        .menu(&menu(app, "افتح زاجِل", "إنهاء زاجِل")?)
        .tooltip("Zajil")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_window(app),
            // The same path as closing the window (`RunEvent::ExitRequested`).
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        });
    #[cfg(target_os = "macos")]
    let tray = tray
        .icon(Image::from_bytes(include_bytes!(
            "../icons/tray-template.png"
        ))?)
        .icon_as_template(true);
    #[cfg(not(target_os = "macos"))]
    let tray = match app.default_window_icon() {
        Some(icon) => tray.icon(Image::clone(icon)),
        None => tray,
    };
    tray.build(app)?;
    Ok(())
}

/// Shows the window and brings it to the front.
pub fn show_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The menu's texts in the interface's language.
pub fn set_texts<R: Runtime>(app: &AppHandle<R>, show: &str, quit: &str) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(ID) {
        tray.set_menu(Some(menu(app, show, quit)?))?;
    }
    Ok(())
}
