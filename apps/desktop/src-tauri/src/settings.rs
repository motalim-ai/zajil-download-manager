//! The settings and categories screens' commands.

use std::sync::atomic::Ordering;

use tauri::{Emitter, State};
use zajil_core::Engine;
use zajil_core::category::CategoriesFile;
use zajil_core::settings::SettingsStore;

use crate::view::{self, CategoryView, SettingsChange, SettingsView};
use crate::{App, blocking_err, in_scene, video};

#[tauri::command]
pub(crate) async fn settings(app: State<'_, App>) -> Result<SettingsView, String> {
    let (settings, store, system) = (
        app.settings.clone(),
        app.store.clone(),
        app.system_download_dir.clone(),
    );
    tokio::task::spawn_blocking(move || {
        let file = settings.load().map_err(blocking_err)?;
        let limit = store.load().map_err(blocking_err)?.settings.speed_limit;
        Ok(view::settings(&file, limit, &system))
    })
    .await
    .map_err(blocking_err)?
}

/// Saves the settings screen: `settings.json`, and the speed limit in the
/// queues file, where the runner reads it within half a second. The next
/// download takes the new number of connections.
#[tauri::command]
pub(crate) async fn save_settings<R: tauri::Runtime>(
    app: State<'_, App>,
    change: SettingsChange,
    handle: tauri::AppHandle<R>,
) -> Result<SettingsView, String> {
    if in_scene(&handle) {
        // The language-switch scene goes through here: applied, not saved.
        let mut file = app.settings.load().unwrap_or_default();
        view::apply_settings(&mut file, &change);
        return Ok(view::settings(
            &file,
            change.speed_limit,
            &app.system_download_dir,
        ));
    }
    let (settings, store, system, connections, clipboard, engine) = (
        app.settings.clone(),
        app.store.clone(),
        app.system_download_dir.clone(),
        app.connections.clone(),
        app.clipboard.clone(),
        app.engine.clone(),
    );
    let saved = tokio::task::spawn_blocking(move || {
        if change.speed_limit == Some(0) {
            return Err("a speed limit must be more than 0".to_owned());
        }
        let file = save_settings_file(&settings, &engine, &change)?;
        store
            .update(|queues| {
                queues.settings.speed_limit = change.speed_limit;
                Ok(())
            })
            .map_err(blocking_err)?;
        connections.store(file.connections as usize, Ordering::Relaxed);
        clipboard.store(file.clipboard, Ordering::Relaxed);
        Ok(view::settings(&file, change.speed_limit, &system))
    })
    .await
    .map_err(blocking_err)??;
    let _ = handle.emit("downloads-changed", ());
    // The other windows follow (the main one and the settings window).
    let _ = handle.emit("settings-changed", saved.clone());
    Ok(saved)
}

/// What a progress window does when its download finishes, for every
/// download (owner's request, 2026-10-30).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct FinishView {
    pub close_window: bool,
    pub open_file: bool,
    pub open_folder: bool,
}

#[tauri::command]
pub(crate) async fn finish_setting(app: State<'_, App>) -> Result<FinishView, String> {
    let settings = app.settings.clone();
    let finish = tokio::task::spawn_blocking(move || settings.load().map(|s| s.finish()))
        .await
        .map_err(blocking_err)?
        .map_err(blocking_err)?;
    Ok(FinishView {
        close_window: finish.close_window,
        open_file: finish.open_file,
        open_folder: finish.open_folder,
    })
}

#[tauri::command]
pub(crate) async fn set_finish_setting(
    app: State<'_, App>,
    finish: FinishView,
) -> Result<(), String> {
    let settings = app.settings.clone();
    tokio::task::spawn_blocking(move || {
        settings.update(|file| {
            file.finish = Some(zajil_core::settings::FinishActions {
                close_window: finish.close_window,
                open_file: finish.open_file,
                open_folder: finish.open_folder,
            });
            Ok(())
        })
    })
    .await
    .map_err(blocking_err)?
    .map_err(blocking_err)
}

/// How many video downloads run at once.
#[tauri::command]
pub(crate) async fn videos_at_once_setting(app: State<'_, App>) -> Result<u32, String> {
    let settings = app.settings.clone();
    tokio::task::spawn_blocking(move || settings.load().map(|s| s.videos_at_once()))
        .await
        .map_err(blocking_err)?
        .map_err(blocking_err)
}

/// Sets how many video downloads run at once, 1 to 8: at once, the
/// waiting ones that may now start start.
#[tauri::command]
pub(crate) async fn set_videos_at_once<R: tauri::Runtime>(
    app: State<'_, App>,
    n: u32,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    let settings = app.settings.clone();
    tokio::task::spawn_blocking(move || {
        settings.update(|file| {
            file.videos_at_once = Some(n);
            Ok(())
        })
    })
    .await
    .map_err(blocking_err)?
    .map_err(blocking_err)?;
    video::set_at_once(&handle, n);
    Ok(())
}

/// The proxy of the settings (`None`: none).
#[tauri::command]
pub(crate) async fn proxy_setting(app: State<'_, App>) -> Result<Option<String>, String> {
    let settings = app.settings.clone();
    tokio::task::spawn_blocking(move || settings.load().map(|s| s.proxy))
        .await
        .map_err(blocking_err)?
        .map_err(blocking_err)
}

/// Sets the proxy (`None` or empty: none). It applies when Zajil next
/// starts: every download then goes through it.
#[tauri::command]
pub(crate) async fn set_proxy(app: State<'_, App>, proxy: Option<String>) -> Result<(), String> {
    let proxy = proxy.map(|p| p.trim().to_owned()).filter(|p| !p.is_empty());
    if let Some(proxy) = &proxy {
        zajil_core::settings::check_proxy(proxy)?;
    }
    let settings = app.settings.clone();
    tokio::task::spawn_blocking(move || {
        settings.update(|file| {
            file.proxy = proxy;
            Ok(())
        })
    })
    .await
    .map_err(blocking_err)?
    .map_err(blocking_err)
}

/// Writes `change` into the settings file and gives its site limits to
/// `engine`, in place, so the running downloads and the queue runner's
/// engine see them at once (#165). A list the file refuses changes
/// neither. One save at a time, so the engine always has the file's list
/// (review of #165: two saves at once could leave it on the older one).
/// Blocking.
fn save_settings_file(
    settings: &SettingsStore,
    engine: &Engine,
    change: &SettingsChange,
) -> Result<zajil_core::settings::SettingsFile, String> {
    static SAVING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = SAVING.lock().unwrap_or_else(|e| e.into_inner());
    let file = settings
        .update(|file| {
            view::apply_settings(file, change);
            Ok(file.clone())
        })
        .map_err(blocking_err)?;
    engine.site_limits().set_from_settings(&file);
    Ok(file)
}

#[tauri::command]
pub(crate) async fn categories(app: State<'_, App>) -> Result<Vec<CategoryView>, String> {
    let categories = app.categories.clone();
    tokio::task::spawn_blocking(move || {
        Ok(view::category_views(
            &categories.load().map_err(blocking_err)?,
        ))
    })
    .await
    .map_err(blocking_err)?
}

/// Replaces the categories with the page's list, checked in full: nothing
/// is written if any value is invalid.
#[tauri::command]
pub(crate) async fn save_categories<R: tauri::Runtime>(
    app: State<'_, App>,
    categories: Vec<CategoryView>,
    handle: tauri::AppHandle<R>,
) -> Result<Vec<CategoryView>, String> {
    if in_scene(&handle) {
        return Err("a ui-test scene saves nothing".into());
    }
    let store = app.categories.clone();
    let saved = tokio::task::spawn_blocking(move || {
        store
            .update(|file| {
                file.categories = view::categories_from(&categories);
                Ok(view::category_views(file))
            })
            .map_err(blocking_err)
    })
    .await
    .map_err(blocking_err)??;
    let _ = handle.emit("downloads-changed", ());
    Ok(saved)
}

/// The category `name` would go to with the page's list, before it is
/// saved ("try a name").
#[tauri::command]
pub(crate) fn try_category(
    categories: Vec<CategoryView>,
    name: String,
) -> Result<Option<String>, String> {
    let file = CategoriesFile {
        categories: view::categories_from(&categories),
        ..CategoriesFile::built_in()
    };
    file.check_consistency()?;
    Ok(file.classify(&name, None).map(|c| c.id.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zajil_core::Options;

    /// #165 (review): saving the settings gives their site list to the
    /// engine that already runs (and its clones); a list the file refuses
    /// changes neither the file nor the engine.
    #[test]
    fn saved_site_limits_reach_the_running_engine_and_a_refused_list_none() {
        let data = tempfile::tempdir().unwrap();
        let settings = SettingsStore::new(data.path());
        let engine = Engine::new(Options::default()).unwrap();
        let running = engine.clone();
        let shown = view::settings(&settings.load().unwrap(), None, data.path());
        let change = |list: Vec<view::SiteView>| view::SettingsChange {
            language: shown.language,
            theme: shown.theme,
            digits: shown.digits,
            download_dir: None,
            connections: shown.connections,
            notifications: shown.notifications,
            clipboard: shown.clipboard,
            update_check: shown.update_check,
            site_connections: list,
            speed_limit: None,
        };
        let site = |name: &str, n| view::SiteView {
            site: name.into(),
            connections: n,
        };
        assert_eq!(running.site_room("https://example.com/f"), None);
        save_settings_file(&settings, &engine, &change(vec![site("example.com", 3)])).unwrap();
        assert_eq!(
            running.site_room("https://example.com/f").map(|r| r.1),
            Some(3)
        );
        let refused = save_settings_file(&settings, &engine, &change(vec![site("*.bad", 2)]));
        assert!(refused.is_err());
        assert_eq!(
            running.site_room("https://example.com/f").map(|r| r.1),
            Some(3)
        );
        assert_eq!(settings.load().unwrap().site_connections.len(), 1);
    }
}
