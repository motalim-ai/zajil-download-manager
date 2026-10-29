//! How a new download shows (M5.5, IDM touches, the settings):
//! - its progress window, when «Download file info» starts it: shown,
//!   minimized, or not shown (the menu and a double-click always show it);
//! - «Start downloading while the window is open»: the download starts in
//!   the background as soon as the server answers, while the user is still
//!   in «Download file info»; «Cancel» then removes it with its files;
//! - «Start browser downloads at once, without asking»: a download from the
//!   browser starts without its «Download file info» (`offers.rs`); a file
//!   that can run code still asks.
//!
//! Kept with the window preferences (`window.json`).

use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;

use crate::closing::Closing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProgressShow {
    #[default]
    Shown,
    Minimized,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AddPrefs {
    #[serde(default)]
    pub progress_window: ProgressShow,
    /// On unless the user turns it off (the owner's request after trying
    /// other download managers); a new name, so an earlier «off» that was
    /// only the old default does not stay.
    #[serde(default = "on")]
    pub start_at_once: bool,
    #[serde(default)]
    pub browser_at_once: bool,
}

fn on() -> bool {
    true
}

impl Default for AddPrefs {
    fn default() -> Self {
        AddPrefs {
            progress_window: ProgressShow::default(),
            start_at_once: true,
            browser_at_once: false,
        }
    }
}

#[tauri::command]
pub fn add_prefs(closing: State<'_, Closing>) -> AddPrefs {
    closing.prefs().add
}

#[tauri::command]
pub fn set_add_prefs(closing: State<'_, Closing>, prefs: AddPrefs) -> Result<(), String> {
    closing.change(|p| p.add = prefs).map_err(|e| e.to_string())
}

/// The progress window of a download «Download file info» started, as the
/// settings say: shown, minimized, or not at all.
#[tauri::command]
pub async fn open_started_progress<R: tauri::Runtime>(
    closing: State<'_, Closing>,
    id: u64,
    title: String,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    if crate::in_scene(&handle) {
        return Ok(());
    }
    show_started(&handle, closing.prefs().add.progress_window, id, &title)
}

/// Download `id`'s progress window, as `show` says. Not on the main
/// thread (`windows::open`).
pub(crate) fn show_started<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    show: ProgressShow,
    id: u64,
    title: &str,
) -> Result<(), String> {
    if show == ProgressShow::Hidden {
        return Ok(());
    }
    let label = crate::windows::progress_label(id);
    crate::windows::open(handle, &label, title, crate::windows::PROGRESS_SIZE)
        .map_err(|e| e.to_string())?;
    if show == ProgressShow::Minimized
        && let Some(window) = crate::windows::find(handle, &label)
    {
        window.minimize().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// At first the progress window shows and the download starts while
    /// «Download file info» is open; a preferences file from before has the
    /// same, even one where the old setting was off.
    #[test]
    fn the_defaults_show_the_progress_and_start_at_once() {
        assert_eq!(
            AddPrefs::default(),
            AddPrefs {
                progress_window: ProgressShow::Shown,
                start_at_once: true,
                browser_at_once: false
            }
        );
        let old: crate::closing::WindowPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(old.add, AddPrefs::default());
        let off: AddPrefs = serde_json::from_str(r#"{"start_while_open":false}"#).unwrap();
        assert!(off.start_at_once);
        let some: AddPrefs = serde_json::from_str(r#"{"progress_window":"minimized"}"#).unwrap();
        assert_eq!(some.progress_window, ProgressShow::Minimized);
    }

    /// A new install's defaults, as decided: only the finished download's
    /// sound, the progress window shown (as in IDM), and the clipboard not
    /// watched (M5).
    #[test]
    fn a_new_install_has_the_decided_defaults() {
        let prefs = crate::closing::WindowPrefs::default();
        assert_eq!(
            (prefs.sounds.done, prefs.sounds.failed, prefs.sounds.queue),
            (true, false, false)
        );
        assert_eq!(prefs.add.progress_window, ProgressShow::Shown);
        assert!(prefs.add.start_at_once);
        // A download from the browser asks first, as in IDM.
        assert!(!prefs.add.browser_at_once);
        assert!(!zajil_core::settings::SettingsFile::default().clipboard);
    }

    /// Only an unfinished download's files are deleted: a finished file
    /// stays.
    #[test]
    fn only_unfinished_files_are_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("a.bin.zajil-state");
        let part = dir.path().join("a.bin.zajil-part");
        std::fs::write(&state, b"{}").unwrap();
        std::fs::write(&part, b"x").unwrap();
        let mut queues = zajil_core::queue::QueuesFile::empty();
        let id = queues
            .add("main", "https://example.com/a.bin", dir.path())
            .unwrap();
        queues.item_mut(id).unwrap().state_file = Some(state.clone());
        let done = queues
            .add("main", "https://example.com/b.bin", dir.path())
            .unwrap();
        queues.item_mut(done).unwrap().file = Some(dir.path().join("b.bin"));
        let files = crate::removal::files_of(&queues, &[id, done]);
        assert_eq!(files, vec![state.clone(), part.clone()]);
        crate::removal::delete_soon(files);
        for _ in 0..50 {
            if !state.exists() && !part.exists() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("the unfinished files were not deleted");
    }
}
