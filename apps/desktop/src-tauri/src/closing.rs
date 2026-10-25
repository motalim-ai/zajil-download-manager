//! Closing the main window (owner's decision, after #105), as in IDM: the
//! window hides and Zajil goes on in the system tray, the downloads with
//! it; the first time, a notification says so. On macOS the program goes on
//! the same way, its icon in the Dock (a click on it shows the window). The
//! setting «Closing the window quits Zajil» and a desktop with no system
//! tray (GNOME without the extension) make closing quit, through the page's
//! Quit, which asks first while downloads run.
//!
//! The two preferences are the app's own (`window.json` in the data
//! folder), not zajil-core's settings, which this change does not touch.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

/// What `window.json` holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPrefs {
    /// Closing the main window quits Zajil.
    #[serde(default)]
    pub close_quits: bool,
    /// The «still running in the tray» notification was shown once.
    #[serde(default)]
    pub tray_notice_shown: bool,
}

/// The texts of that notification, in the page's language.
#[derive(Debug, Clone, Default)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

pub struct Closing {
    path: PathBuf,
    prefs: Mutex<WindowPrefs>,
    notice: Mutex<Notice>,
}

const FILE: &str = "window.json";

impl Closing {
    pub fn new(data_dir: &Path) -> Self {
        let path = data_dir.join(FILE);
        Closing {
            prefs: Mutex::new(load(&path)),
            path,
            notice: Mutex::default(),
        }
    }

    pub fn prefs(&self) -> WindowPrefs {
        self.prefs.lock().unwrap().clone()
    }

    /// Changes the preferences and keeps them.
    pub fn change(&self, change: impl FnOnce(&mut WindowPrefs)) -> std::io::Result<()> {
        let mut prefs = self.prefs.lock().unwrap();
        change(&mut prefs);
        save(&self.path, &prefs)
    }

    pub fn set_notice(&self, notice: Notice) {
        *self.notice.lock().unwrap() = notice;
    }
}

/// Unreadable or missing: the defaults (closing hides).
fn load(path: &Path) -> WindowPrefs {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Written whole, then put in place.
fn save(path: &Path, prefs: &WindowPrefs) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(prefs).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, path)
}

/// What closing the main window does.
#[derive(Debug, PartialEq, Eq)]
pub enum OnClose {
    /// Hide it; Zajil goes on (with the notice the first time).
    Hide { notice: bool },
    /// Quit, through the page's Quit.
    Quit,
}

pub fn on_close(prefs: &WindowPrefs, tray: bool) -> OnClose {
    if prefs.close_quits || !tray {
        OnClose::Quit
    } else {
        OnClose::Hide {
            notice: !prefs.tray_notice_shown,
        }
    }
}

/// Whether the desktop shows tray icons. Windows and macOS always do; on
/// Linux, only with a StatusNotifier host (KDE, most desktops; GNOME only
/// with its extension).
pub fn has_tray() -> bool {
    #[cfg(target_os = "linux")]
    {
        let ask = || -> zbus::Result<bool> {
            let bus = zbus::blocking::Connection::session()?;
            let dbus = zbus::blocking::fdo::DBusProxy::new(&bus)?;
            Ok(dbus.name_has_owner("org.kde.StatusNotifierWatcher".try_into()?)?)
        };
        ask().unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    true
}

/// Handles the main window's close button.
pub fn watch<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    let Some(main) = handle.get_webview_window("main") else {
        return;
    };
    let app = handle.clone();
    let window = main.clone();
    main.on_window_event(move |event| {
        let tauri::WindowEvent::CloseRequested { api, .. } = event else {
            return;
        };
        // Quitting already (the page's Quit, the tray's, a signal): close.
        if app
            .state::<crate::App>()
            .exiting
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return;
        }
        api.prevent_close();
        let closing = app.state::<Closing>();
        match on_close(&closing.prefs(), has_tray()) {
            OnClose::Quit => {
                let _ = app.emit_to("main", "menu-command", "quit");
            }
            OnClose::Hide { notice } => {
                let _ = window.hide();
                if notice {
                    show_notice(&app, &closing);
                    let _ = closing.change(|p| p.tray_notice_shown = true);
                }
            }
        }
    });
}

fn show_notice<R: tauri::Runtime>(app: &tauri::AppHandle<R>, closing: &Closing) {
    use tauri_plugin_notification::NotificationExt;
    let notice = closing.notice.lock().unwrap().clone();
    if notice.body.is_empty() {
        return;
    }
    let _ = app
        .notification()
        .builder()
        .title(notice.title)
        .body(notice.body)
        .show();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_hides_unless_the_user_or_the_desktop_says_quit() {
        let mut prefs = WindowPrefs::default();
        assert_eq!(on_close(&prefs, true), OnClose::Hide { notice: true });
        prefs.tray_notice_shown = true;
        assert_eq!(on_close(&prefs, true), OnClose::Hide { notice: false });
        // No system tray: nowhere to go on.
        assert_eq!(on_close(&prefs, false), OnClose::Quit);
        prefs.close_quits = true;
        assert_eq!(on_close(&prefs, true), OnClose::Quit);
    }

    #[test]
    fn the_preferences_are_kept_and_a_damaged_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let closing = Closing::new(dir.path());
        assert_eq!(closing.prefs(), WindowPrefs::default());
        closing.change(|p| p.close_quits = true).unwrap();
        assert!(Closing::new(dir.path()).prefs().close_quits);
        std::fs::write(dir.path().join(FILE), b"{not json").unwrap();
        assert_eq!(Closing::new(dir.path()).prefs(), WindowPrefs::default());
    }
}
