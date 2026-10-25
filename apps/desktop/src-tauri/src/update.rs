//! «A new version of Zajil is available» (#124, owner's decision): once at
//! each start, after the window shows, if the settings allow it, Zajil asks
//! GitHub for its newest release (`zajil_core::update`), never downloading
//! or installing anything. Not in a development build, whose version
//! (0.0.0) would always find one. A failure (no network, GitHub's limit,
//! an odd answer) is one line in the log, nothing for the user. The page
//! shows a bar; «Release page» opens the release's page from Rust, the page
//! never holding a link.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};
use ts_rs::TS;
use zajil_core::update::{CheckError, NewRelease};

use crate::App;

/// The newer release found at this start, if any.
#[derive(Debug, Default)]
pub struct Notice(Mutex<Option<NewRelease>>);

/// What the page shows of a newer release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpdateView {
    pub version: String,
}

/// The check at start, in the background: the window is not kept waiting.
pub fn check_at_start(handle: &tauri::AppHandle) {
    // A development build is 0.0.0: every release would be newer.
    if cfg!(debug_assertions) {
        return;
    }
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let (settings, engine) = {
            let app = handle.state::<App>();
            (app.settings.clone(), app.engine.clone())
        };
        let on = tokio::task::spawn_blocking(move || settings.load().map(|s| s.update_check))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(true);
        let current = handle.package_info().version.to_string();
        let stop = zajil_core::Stop::new();
        let found = found(on, || {
            zajil_core::update::newer_release(&engine, &current, &stop)
        })
        .await;
        if let Some(release) = found {
            let view = UpdateView {
                version: release.version.clone(),
            };
            *handle.state::<Notice>().0.lock().unwrap() = Some(release);
            let _ = handle.emit_to("main", "update-available", view);
        }
    });
}

/// The newer release `check` finds, when the check is `on`; nothing is
/// asked when it is off, and a failure is only logged.
async fn found<F, Fut>(on: bool, check: F) -> Option<NewRelease>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Option<NewRelease>, CheckError>>,
{
    if !on {
        return None;
    }
    match check().await {
        Ok(found) => found,
        Err(err) => {
            eprintln!("zajil: the update check: {err}");
            None
        }
    }
}

/// The newer release found at this start, for a page that opens later.
#[tauri::command]
pub fn update_notice(notice: State<'_, Notice>) -> Option<UpdateView> {
    notice.0.lock().unwrap().as_ref().map(|r| UpdateView {
        version: r.version.clone(),
    })
}

/// «Release page»: the found release's page in the default browser.
#[tauri::command]
pub fn open_release_page<R: tauri::Runtime>(
    notice: State<'_, Notice>,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let Some(page) = notice.0.lock().unwrap().as_ref().map(|r| r.page.clone()) else {
        return Err("no newer release was found".into());
    };
    handle
        .opener()
        .open_url(page, None::<&str>)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release() -> NewRelease {
        NewRelease {
            version: "0.2.0".into(),
            page: "https://github.com/motalim-ai/zajil-download-manager/releases/tag/v0.2.0".into(),
        }
    }

    /// #124: the notice comes with a newer release, and not with none or
    /// a failure; when the setting is off, nothing is asked at all.
    #[tokio::test]
    async fn the_notice_follows_the_check_and_the_setting() {
        assert_eq!(
            found(true, || async { Ok(Some(release())) }).await,
            Some(release())
        );
        assert_eq!(found(true, || async { Ok(None) }).await, None);
        let failed = found(true, || async { Err(CheckError::Answer("odd".into())) }).await;
        assert_eq!(failed, None);
        let asked = std::sync::atomic::AtomicBool::new(false);
        let off = found(false, || async {
            asked.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(Some(release()))
        })
        .await;
        assert_eq!(off, None);
        assert!(
            !asked.load(std::sync::atomic::Ordering::SeqCst),
            "nothing asked"
        );
    }
}
