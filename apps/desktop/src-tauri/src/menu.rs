//! The menu bar on macOS (M5.5, the design of #83): there the menus are in
//! the system's bar at the top of the screen, not in the window (on Windows
//! and Linux the page draws its own). The page gives the menus in its
//! language ([`set_menu`]): each item is one of the page's commands, and a
//! click comes back to the page as the `menu-command` event with its id.
//! The Edit menu keeps the system's own Undo, Cut, Copy and Paste, without
//! which text boxes cannot use them on macOS, and a Window menu has
//! Minimize and Zoom.

use serde::Deserialize;
use tauri::{AppHandle, Runtime};

/// A menu, as the page gives it.
#[derive(Debug, Deserialize)]
// Read only where the system's bar is built (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct MenuSpec {
    pub label: String,
    /// The Edit menu: the system's own text items go first.
    #[serde(default)]
    pub edit: bool,
    pub items: Vec<ItemSpec>,
}

/// One of its items: a command, or a separator (no id).
#[derive(Debug, Deserialize)]
// Read only where the system's bar is built (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct ItemSpec {
    pub id: Option<String>,
    #[serde(default)]
    pub label: String,
    /// As Tauri writes it (`CmdOrCtrl+Shift+P`).
    #[serde(default)]
    pub accelerator: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// The system's own items, named in the page's language.
#[derive(Debug, Deserialize)]
// Read only where the system's bar is built (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct SystemTexts {
    pub hide: String,
    pub hide_others: String,
    pub show_all: String,
    pub undo: String,
    pub redo: String,
    pub cut: String,
    pub copy: String,
    pub paste: String,
    pub window: String,
    pub minimize: String,
    pub zoom: String,
}

/// The prefix of the page's command ids in the menu (the tray's own items
/// have others).
pub const PREFIX: &str = "cmd:";

/// Sets the macOS menu bar; nothing elsewhere.
#[tauri::command]
pub fn set_menu<R: Runtime>(
    handle: AppHandle<R>,
    app_name: String,
    menus: Vec<MenuSpec>,
    system: SystemTexts,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let menu = build(&handle, &app_name, &menus, &system).map_err(|e| e.to_string())?;
        handle.set_menu(menu).map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (handle, app_name, menus, system);
    Ok(())
}

#[cfg(target_os = "macos")]
fn build<R: Runtime>(
    app: &AppHandle<R>,
    app_name: &str,
    menus: &[MenuSpec],
    system: &SystemTexts,
) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
    let items = |spec: &[ItemSpec]| -> tauri::Result<Vec<Box<dyn IsMenuItem<R>>>> {
        let mut out: Vec<Box<dyn IsMenuItem<R>>> = Vec::new();
        for item in spec {
            match &item.id {
                None => out.push(Box::new(PredefinedMenuItem::separator(app)?)),
                Some(id) => out.push(Box::new(MenuItem::with_id(
                    app,
                    format!("{PREFIX}{id}"),
                    &item.label,
                    item.enabled,
                    item.accelerator.as_deref(),
                )?)),
            }
        }
        Ok(out)
    };
    // The app's own menu: Hide, Hide Others, Show All, then Quit (a command
    // of the page, so quitting goes the way the tray's does).
    let mut first: Vec<Box<dyn IsMenuItem<R>>> = vec![
        Box::new(PredefinedMenuItem::hide(app, Some(&system.hide))?),
        Box::new(PredefinedMenuItem::hide_others(
            app,
            Some(&system.hide_others),
        )?),
        Box::new(PredefinedMenuItem::show_all(app, Some(&system.show_all))?),
        Box::new(PredefinedMenuItem::separator(app)?),
    ];
    let mut rest = Vec::new();
    let mut has_quit = false;
    for menu in menus {
        let mut list = items(&menu.items)?;
        if menu.edit {
            let mut own: Vec<Box<dyn IsMenuItem<R>>> = vec![
                Box::new(PredefinedMenuItem::undo(app, Some(&system.undo))?),
                Box::new(PredefinedMenuItem::redo(app, Some(&system.redo))?),
                Box::new(PredefinedMenuItem::separator(app)?),
                Box::new(PredefinedMenuItem::cut(app, Some(&system.cut))?),
                Box::new(PredefinedMenuItem::copy(app, Some(&system.copy))?),
                Box::new(PredefinedMenuItem::paste(app, Some(&system.paste))?),
                Box::new(PredefinedMenuItem::separator(app)?),
            ];
            own.append(&mut list);
            list = own;
        }
        // The page's quit goes last in the app menu, not in File.
        if let Some(at) = menu
            .items
            .iter()
            .position(|i| i.id.as_deref() == Some("quit"))
        {
            let start = list.len() - menu.items.len();
            let quit = list.remove(start + at);
            first.push(quit);
            has_quit = true;
            // No separator left at the menu's end.
            if at + 1 == menu.items.len() && at > 0 && menu.items[at - 1].id.is_none() {
                list.remove(start + at - 1);
            }
        }
        let refs: Vec<&dyn IsMenuItem<R>> = list.iter().map(|b| b.as_ref()).collect();
        rest.push(Submenu::with_items(app, &menu.label, true, &refs)?);
    }
    // The page gave no Quit: the system's, so ⌘Q always quits.
    if !has_quit {
        first.push(Box::new(PredefinedMenuItem::quit(app, None)?));
    }
    let first_refs: Vec<&dyn IsMenuItem<R>> = first.iter().map(|b| b.as_ref()).collect();
    let app_menu = Submenu::with_items(app, app_name, true, &first_refs)?;
    let window = Submenu::with_items(
        app,
        &system.window,
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some(&system.minimize))?,
            &PredefinedMenuItem::maximize(app, Some(&system.zoom))?,
        ],
    )?;
    let mut all: Vec<&dyn IsMenuItem<R>> = vec![&app_menu];
    for menu in &rest {
        all.push(menu);
    }
    all.push(&window);
    Menu::with_items(app, &all)
}

/// Which window a macOS menu command goes to, with `front` the window in
/// front: Select all is the front window's own (in an add window, its text
/// box's, M5.5); every other command is the main window's, which alone
/// handles them (#98), Quit among them, which asks first while downloads
/// run (review of #105).
pub(crate) fn menu_target<'a>(command: &str, front: &'a str) -> &'a str {
    if command == "select-all" {
        front
    } else {
        "main"
    }
}

/// The program's version, for «About Zajil».
#[tauri::command]
pub(crate) fn app_version<R: tauri::Runtime>(handle: tauri::AppHandle<R>) -> String {
    handle.package_info().version.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #98: the macOS menu's commands reach the main window from any
    /// window in front, but Select all stays the front window's.
    #[test]
    fn menu_commands_go_to_the_main_window_but_select_all() {
        assert_eq!(menu_target("select-all", "add-1"), "add-1");
        assert_eq!(menu_target("select-all", "main"), "main");
        for command in ["add-link", "settings", "quit", "pause-all", "schedule"] {
            assert_eq!(menu_target(command, "progress-7"), "main", "{command}");
            assert_eq!(menu_target(command, "main"), "main", "{command}");
        }
    }
}
