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
use tauri::{AppHandle, Emitter, Manager, Runtime};

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
            // The page's Quit, which asks first while downloads run (owner's
            // decision on #83), its window shown for the question; without
            // the window, the same path as closing it.
            "quit" => match app.get_webview_window("main") {
                Some(_) => {
                    show_window(app);
                    let _ = app.emit_to("main", "menu-command", "quit");
                }
                None => app.exit(0),
            },
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

/// The longest tooltip Windows shows (`NOTIFYICONDATAW::szTip` holds 128
/// UTF-16 units with its end).
const TIP_UNITS: usize = 127;

/// What runs now, on the icon's tooltip (from the page, in its language).
/// Windows and macOS show it; Linux's tray (AppIndicator) has no tooltip.
#[tauri::command]
pub fn set_tray_tooltip<R: Runtime>(handle: AppHandle<R>, text: String) -> Result<(), String> {
    if let Some(tray) = handle.tray_by_id(ID) {
        tray.set_tooltip(Some(fit(&text)))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `text` cut to [`TIP_UNITS`] UTF-16 units, at a character's edge.
fn fit(text: &str) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= TIP_UNITS
        })
        .collect()
}

/// The menu's texts in the interface's language.
pub fn set_texts<R: Runtime>(app: &AppHandle<R>, show: &str, quit: &str) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(ID) {
        tray.set_menu(Some(menu(app, show, quit)?))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_tooltip_is_cut_at_a_character_within_windows_limit() {
        assert_eq!(fit("زاجِل"), "زاجِل");
        let long = "ب".repeat(200);
        let cut = fit(&long);
        assert_eq!(cut.encode_utf16().count(), TIP_UNITS);
        // A character of two UTF-16 units is never cut in half.
        let wide = "😀".repeat(100);
        let cut = fit(&wide);
        assert_eq!(cut.encode_utf16().count(), TIP_UNITS - 1);
        assert!(cut.chars().all(|c| c == '😀'));
    }
}
