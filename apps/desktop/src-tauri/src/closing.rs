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
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

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
    /// The drop box is shown (`dropbox.rs`); off at first.
    #[serde(default)]
    pub drop_box: bool,
    /// Where the user left it.
    #[serde(default)]
    pub drop_place: Option<crate::dropbox::Place>,
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
    /// Whether there is a tray: looked for once, off the window's thread
    /// (review of #109: a D-Bus round trip at every close). 0: not known
    /// yet, 1: yes, 2: no.
    tray: std::sync::Arc<AtomicU8>,
    /// The page answered the «quit» a close sent it.
    heard: AtomicBool,
    /// `--close-quits`.
    forced: bool,
}

/// How long the page has to answer a close's «quit» before Zajil quits
/// without it (a page not loaded or stuck: review of #109).
const PAGE_ANSWER: Duration = Duration::from_secs(3);

const FILE: &str = "window.json";

impl Closing {
    /// `close_quits`: `--close-quits` was given (the tests, which close
    /// the window to quit it on Windows): closing quits at once, with no
    /// question, whatever the setting says, and nothing is saved.
    pub fn new(data_dir: &Path, close_quits: bool) -> Self {
        let path = data_dir.join(FILE);
        let tray = std::sync::Arc::new(AtomicU8::new(0));
        let found = tray.clone();
        std::thread::spawn(move || found.store(if has_tray() { 1 } else { 2 }, Ordering::SeqCst));
        Closing {
            prefs: Mutex::new(load(&path)),
            path,
            forced: close_quits,
            notice: Mutex::default(),
            tray,
            heard: AtomicBool::new(false),
        }
    }

    /// Whether the desktop shows tray icons (looked for at start).
    pub fn tray(&self) -> bool {
        match self.tray.load(Ordering::SeqCst) {
            1 => true,
            2 => false,
            _ => {
                let found = has_tray();
                self.tray.store(if found { 1 } else { 2 }, Ordering::SeqCst);
                found
            }
        }
    }

    pub fn heard(&self) {
        self.heard.store(true, Ordering::SeqCst);
    }

    pub fn prefs(&self) -> WindowPrefs {
        let mut prefs = self.prefs.lock().unwrap().clone();
        prefs.close_quits |= self.forced;
        prefs
    }

    /// Changes the preferences and keeps them; nothing changes if they
    /// cannot be kept (review of #109: the page and Rust disagreed).
    pub fn change(&self, change: impl FnOnce(&mut WindowPrefs)) -> std::io::Result<()> {
        let mut prefs = self.prefs.lock().unwrap();
        let mut next = prefs.clone();
        change(&mut next);
        save(&self.path, &next)?;
        *prefs = next;
        Ok(())
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
        // `--close-quits` (the tests, and any script that closes the
        // window to quit Zajil): the window closes as before #109, and the
        // program ends the way closing always ended it, the downloads
        // recording their progress first; no question, which nobody would
        // answer.
        if app.state::<Closing>().forced {
            return;
        }
        api.prevent_close();
        let closing = app.state::<Closing>();
        match on_close(&closing.prefs(), closing.tray()) {
            OnClose::Quit => {
                // The page's Quit asks first while downloads run; a page
                // that does not answer does not keep Zajil open.
                closing.heard.store(false, Ordering::SeqCst);
                let _ = app.emit_to("main", "menu-command", "quit");
                let app = app.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(PAGE_ANSWER);
                    if !app.state::<Closing>().heard.load(Ordering::SeqCst) {
                        app.exit(0);
                    }
                });
            }
            OnClose::Hide { notice } => {
                let _ = window.hide();
                // Kept as said only once it was (review of #109: before
                // the page gave the texts, it was lost for good).
                if notice && show_notice(&app, &closing) {
                    let _ = closing.change(|p| p.tray_notice_shown = true);
                }
            }
        }
    });
}

/// Whether the notice went.
fn show_notice<R: tauri::Runtime>(app: &tauri::AppHandle<R>, closing: &Closing) -> bool {
    use tauri_plugin_notification::NotificationExt;
    let notice = closing.notice.lock().unwrap().clone();
    if notice.body.is_empty() {
        return false;
    }
    app.notification()
        .builder()
        .title(notice.title)
        .body(notice.body)
        .show()
        .is_ok()
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
        let closing = Closing::new(dir.path(), false);
        assert_eq!(closing.prefs(), WindowPrefs::default());
        closing.change(|p| p.close_quits = true).unwrap();
        assert!(Closing::new(dir.path(), false).prefs().close_quits);
        std::fs::write(dir.path().join(FILE), b"{not json").unwrap();
        assert_eq!(
            Closing::new(dir.path(), false).prefs(),
            WindowPrefs::default()
        );
        // `--close-quits`: closing quits, and nothing is written.
        let forced = Closing::new(dir.path(), true);
        assert!(forced.prefs().close_quits);
        assert_eq!(std::fs::read(dir.path().join(FILE)).unwrap(), b"{not json");
    }

    /// A change that cannot be kept changes nothing.
    #[test]
    fn a_change_that_cannot_be_kept_is_not_made() {
        let dir = tempfile::tempdir().unwrap();
        let closing = Closing::new(&dir.path().join("missing"), false);
        assert!(closing.change(|p| p.close_quits = true).is_err());
        assert!(!closing.prefs().close_quits);
    }
}
