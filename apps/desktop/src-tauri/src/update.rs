//! «A new version of Zajil is available» (#124, owner's decision): once at
//! each start, after the window shows, if the settings allow it, Zajil asks
//! GitHub for its newest release (`zajil_core::update`). Not in a
//! development build, whose version (0.0.0) would always find one. A
//! failure (no network, GitHub's limit, an odd answer) is one line in the
//! log, nothing for the user. The page shows a bar; «Release page» opens
//! the release's page from Rust, the page never holding a link.
//!
//! The signed update (ADR-0011, owner's request of 2026-10-31): the newest
//! release's `latest.json` names, for this system, its file and that
//! file's signature. Zajil offers «Update now» only when the signature is
//! by one of the project's two keys built into it, and the file is in the
//! project's own release downloads. Nothing is downloaded before the user
//! says so; then the file is checked by our own code with that key, and
//! again by Tauri's updater (which also checks the version it was signed
//! for), the downloads stop after saving their progress, and Zajil is
//! replaced and opened again. A `.deb` has no file in `latest.json`: the
//! notice and the release page only. «Skip this version» hides that one.

use std::sync::Mutex;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use ts_rs::TS;
use zajil_core::update::{CheckError, NewRelease};

use crate::App;

/// The newer release found at this start, if any, and its signed update
/// when this system has one.
#[derive(Default)]
pub struct Notice(Mutex<Option<NewRelease>>, Mutex<Option<Update>>);

/// What the page shows of a newer release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpdateView {
    pub version: String,
    /// It can be installed from here («Update now»): a signed update.
    pub can_install: bool,
}

/// How far the update's download is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpdateProgressView {
    #[ts(type = "number")]
    pub done: u64,
    #[ts(type = "number | null")]
    pub total: Option<u64>,
}

/// The newest release's update manifest (ADR-0011).
const MANIFEST: &str =
    "https://github.com/motalim-ai/zajil-download-manager/releases/latest/download/latest.json";
/// The only place an update's file may come from.
const DOWNLOADS: &str = "https://github.com/motalim-ai/zajil-download-manager/releases/download/";

/// The project's two public keys (ADR-0011, section 3), as `tauri signer
/// generate` writes them: the signing key, then the offline emergency key.
/// Empty: no update is offered with it.
const KEYS: [(&str, &str); 2] = [
    ("signing", include_str!("../update-keys/signing.pub")),
    ("emergency", include_str!("../update-keys/emergency.pub")),
];

/// The 8-byte key id in a minisign public key or signature, as Tauri
/// writes them (base64 of the minisign text): its second line, after the
/// 2-byte algorithm. None when it is not one.
fn key_id(tauri_text: &str) -> Option<[u8; 8]> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let text = b64.decode(tauri_text.trim()).ok()?;
    let text = std::str::from_utf8(&text).ok()?;
    let line = text.lines().nth(1)?;
    let bytes = b64.decode(line.trim()).ok()?;
    bytes.get(2..10)?.try_into().ok()
}

/// The key of ours `signature` was made with: its name and public key.
fn key_for(
    signature: &str,
    keys: &[(&'static str, &'static str)],
) -> Option<(&'static str, &'static str)> {
    let id = key_id(signature)?;
    keys.iter()
        .copied()
        .find(|(_, key)| !key.trim().is_empty() && key_id(key) == Some(id))
}

/// Our own check (ADR-0011): `bytes` signed with `key`, both as Tauri
/// writes them.
fn signed_by(bytes: &[u8], signature: &str, key: &str) -> bool {
    let b64 = base64::engine::general_purpose::STANDARD;
    let text = |t: &str| {
        b64.decode(t.trim())
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
    };
    let (Some(signature), Some(key)) = (text(signature), text(key)) else {
        return false;
    };
    let (Ok(signature), Ok(key)) = (
        minisign_verify::Signature::decode(&signature),
        minisign_verify::PublicKey::decode(&key),
    ) else {
        return false;
    };
    key.verify(bytes, &signature, false).is_ok()
}

/// The signed update of this release for this system, if any: by a key of
/// ours, from the project's own downloads. Its absence is a line in the
/// log at most.
async fn signed_update<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) -> Option<Update> {
    let (_, first) = KEYS
        .iter()
        .copied()
        .find(|(_, key)| !key.trim().is_empty())?;
    let check = |key: &str| {
        let built = MANIFEST
            .parse::<tauri::Url>()
            .map_err(|e| e.to_string())
            .and_then(|url| {
                handle
                    .updater_builder()
                    .endpoints(vec![url])
                    .and_then(|b| b.pubkey(key).build())
                    .map_err(|e| e.to_string())
            });
        async move {
            match built {
                Ok(updater) => updater.check().await.map_err(|e| e.to_string()),
                Err(e) => Err(e),
            }
        }
    };
    let update = match check(first).await {
        Ok(update) => update?,
        // A system with no file in it (a `.deb`) says so here too.
        Err(e) => {
            eprintln!("zajil: no signed update to install: {e}");
            return None;
        }
    };
    let Some((name, key)) = key_for(&update.signature, &KEYS) else {
        eprintln!("zajil: the update is not signed by a key of Zajil's; not offered");
        return None;
    };
    if !update.download_url.as_str().starts_with(DOWNLOADS) {
        eprintln!("zajil: the update's file is not in Zajil's own releases; not offered");
        return None;
    }
    // Tauri's own check is then made with that key.
    let update = if key == first {
        update
    } else {
        eprintln!("zajil: the update is signed with the {name} key");
        check(key).await.ok().flatten()?
    };
    Some(update)
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
        let Some(release) = found else { return };
        let settings = handle.state::<App>().settings.clone();
        let skipped = tokio::task::spawn_blocking(move || settings.load().map(|s| s.skip_update))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();
        if skipped.as_deref() == Some(release.version.as_str()) {
            return;
        }
        let update = signed_update(&handle).await;
        let view = UpdateView {
            version: release.version.clone(),
            can_install: update.is_some(),
        };
        let notice = handle.state::<Notice>();
        *notice.0.lock().unwrap() = Some(release);
        *notice.1.lock().unwrap() = update;
        let _ = handle.emit_to("main", "update-available", view);
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
    let can_install = notice.1.lock().unwrap().is_some();
    notice.0.lock().unwrap().as_ref().map(|r| UpdateView {
        version: r.version.clone(),
        can_install,
    })
}

/// «Skip this version»: not shown again at the next starts; a newer one is.
#[tauri::command]
pub async fn update_skip<R: tauri::Runtime>(handle: tauri::AppHandle<R>) -> Result<(), String> {
    let notice = handle.state::<Notice>();
    let Some(version) = notice.0.lock().unwrap().take().map(|r| r.version) else {
        return Ok(());
    };
    notice.1.lock().unwrap().take();
    let settings = handle.state::<App>().settings.clone();
    tokio::task::spawn_blocking(move || {
        settings.update(|file| {
            file.skip_update = Some(version);
            Ok(())
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

/// «Update now»: the file downloaded (its progress as `update-progress`),
/// checked by our own code with the key that signed it, then the
/// downloads stop after saving their progress, and Tauri's updater checks
/// it again, installs it, and Zajil opens again. On Windows the installer
/// ends Zajil itself and opens it again.
#[tauri::command]
pub async fn update_install<R: tauri::Runtime>(handle: tauri::AppHandle<R>) -> Result<(), String> {
    let Some(update) = handle.state::<Notice>().1.lock().unwrap().clone() else {
        return Err("no signed update was found".into());
    };
    let mut done = 0u64;
    let mut shown = std::time::Instant::now();
    let events = handle.clone();
    let bytes = update
        .download(
            |chunk, total| {
                done += chunk as u64;
                if shown.elapsed() >= std::time::Duration::from_millis(200) {
                    shown = std::time::Instant::now();
                    let _ = events.emit_to(
                        "main",
                        "update-progress",
                        UpdateProgressView { done, total },
                    );
                }
            },
            || {},
        )
        .await
        .map_err(|e| {
            eprintln!("zajil: the update could not be downloaded or checked: {e}");
            e.to_string()
        })?;
    let ours = key_for(&update.signature, &KEYS)
        .is_some_and(|(_, key)| signed_by(&bytes, &update.signature, key));
    if !ours {
        eprintln!("zajil: the update's file is not signed by a key of Zajil's; not installed");
        return Err("the update could not be checked".into());
    }
    eprintln!("zajil: installing the update to {}", update.version);
    // As when quitting: the downloads save their progress and stop, and
    // continue after the update.
    {
        let app = handle.state::<App>();
        app.exiting.store(true, std::sync::atomic::Ordering::SeqCst);
        app.stop.request();
        crate::wait_for_queues(app.stopped.subscribe(), crate::STOP_WAIT).await;
    }
    if let Err(e) = update.install(&bytes) {
        // Opened again as it was: the bar comes back at its start.
        eprintln!("zajil: the update could not be installed: {e}");
    }
    handle.restart()
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

    // Throwaway test keys (`tauri signer generate`, their private halves
    // not kept), and `FILE` signed with key A for version 9.9.9.
    const KEY_A: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDY3QkQ3NTgxMDRCQ0Y1NkYKUldSdjlid0VnWFc5WjFkMkk0U1QxVEdTbVVnczhSeEhiN3MzdURWWHgvcFlpR2JwK0NkVnU0NksK";
    const KEY_B: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDcwNDE0ODg0RTExQzlCNkQKUldSdG14emhoRWhCY0NxUXp3TEEyeU1tdVA1bTJabkpFdVV3MzJjYkhMNVhpS1ErL0lJbCtUOHgK";
    const FILE: &[u8] = b"zajil update test file\n";
    const SIGNED_BY_A: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSdjlid0VnWFc5Wnd4RXUwZjNPVTFBSm1kcHJML2xLbWxDMkQzclpwZWsxNE9leTI0WkNGZytPQjUyQkVIUHBzbmNHajNCZnFKa3pKbHpVajFvZHNzTUxwL2EvUmFsckFFPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzMzk2OTYwCWZpbGU6ZmlsZS5iaW4JdmVyc2lvbjo5LjkuOQpTTUtTNWhqTWx1SU1qRDIvLzR4ZTFqcjUyR2ZlQzVCRlVZakJPVi84V1plMDBheDNzc3ByeGk2Z0dIWTFYMmRTK21hcFpSTDdYaDgvaVlLOGRoYW9Bdz09Cg==";

    /// ADR-0011: an update is offered and installed only when signed by a
    /// key of ours, the emergency one too; never with a key that is not
    /// there, nor a file changed after it was signed.
    #[test]
    fn only_a_file_signed_by_one_of_our_keys_is_taken() {
        assert_eq!(
            key_for(SIGNED_BY_A, &[("signing", KEY_A), ("emergency", KEY_B)]),
            Some(("signing", KEY_A))
        );
        assert_eq!(
            key_for(SIGNED_BY_A, &[("signing", KEY_B), ("emergency", KEY_A)]),
            Some(("emergency", KEY_A))
        );
        assert_eq!(
            key_for(SIGNED_BY_A, &[("signing", KEY_B), ("emergency", "")]),
            None
        );
        assert_eq!(key_for("not a signature", &[("signing", KEY_A)]), None);
        assert!(signed_by(FILE, SIGNED_BY_A, KEY_A));
        assert!(!signed_by(FILE, SIGNED_BY_A, KEY_B));
        assert!(!signed_by(b"zajil update test filE\n", SIGNED_BY_A, KEY_A));
        assert!(!signed_by(FILE, "", KEY_A));
    }

    /// Until the owner's keys are in, no update is offered to install.
    #[test]
    fn the_built_in_keys_are_public_keys_or_none() {
        for (name, key) in KEYS {
            assert!(
                key.trim().is_empty() || key_id(key).is_some(),
                "update-keys/{name}.pub is not a public key"
            );
            assert!(
                !key.contains("secret") && !key.contains("SECRET"),
                "{name}: a private key"
            );
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
