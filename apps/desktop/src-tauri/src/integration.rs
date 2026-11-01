//! The browser integration: `zajil-host` registered with every
//! supported browser for the current user (ADR-0005).

use std::path::{Path, PathBuf};

use tauri::State;
use zajil_platform::native_host::{Browser, NativeHostRegistrar, SystemRegistrar};

use crate::view::{self};
use crate::{App, blocking_err, in_scene};

/// `zajil-host` beside the program `exe` (the installers put it there; in a
/// development build, `cargo build -p zajil-host` run in
/// `apps/desktop/src-tauri` does), if a browser may safely be told to start
/// it: on macOS and Linux the host is a program (an execute bit) that only
/// its owner, the user or root, can change, in a folder only they can
/// change.
///
/// `exe`'s real path (links resolved) must not be in a place that goes away
/// when the program closes. From an AppImage (its mount, `.mount_…`, or
/// `appimage`: `APPIMAGE` is set), the host beside it is copied to `fixed`
/// in Zajil's data folder, and that copy is the one given to browsers,
/// kept the same as the AppImage's (#129, `native_host::fixed_copy`).
/// macOS's App Translocation is still refused: the user moves the app. On
/// Windows, a folder other users may change (a second drive, #334) gets
/// the same copy; on macOS and Linux it is refused.
fn host_beside(exe: &Path, appimage: bool, fixed: &Path) -> Result<PathBuf, view::HostProblemView> {
    use view::HostProblemView as Problem;
    // Links resolved on macOS and Linux (a CLI or Homebrew link). Not on
    // Windows: there the real path takes a `\\?\` form browsers may not
    // start, and the installers make no links.
    #[cfg(unix)]
    let exe = std::fs::canonicalize(exe).map_err(|_| Problem::Missing)?;
    #[cfg(windows)]
    let exe = exe.to_path_buf();
    let part_named = |is: fn(&str) -> bool| {
        exe.components()
            .any(|part| is(&part.as_os_str().to_string_lossy()))
    };
    if part_named(|part| part == "AppTranslocation") {
        return Err(Problem::Temporary);
    }
    let appimage = appimage || part_named(|part| part.starts_with(".mount_"));
    let dir = exe.parent().ok_or(Problem::Missing)?;
    let host = dir.join(if cfg!(windows) {
        "zajil-host.exe"
    } else {
        "zajil-host"
    });
    let meta = host.symlink_metadata().map_err(|_| Problem::Missing)?;
    if !meta.is_file() {
        return Err(Problem::Missing);
    }
    if meta.len() == 0 {
        return Err(Problem::Missing);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err(Problem::Missing);
        }
    }
    if appimage {
        // The copy is checked instead (its folder, itself, its SHA-256).
        return zajil_platform::native_host::fixed_copy(&host, fixed).map_err(|err| {
            eprintln!("zajil: the browsers' copy of zajil-host: {err}");
            if err.kind() == std::io::ErrorKind::PermissionDenied {
                Problem::Unsafe
            } else {
                Problem::Missing
            }
        });
    }
    let safe =
        |path: &Path| zajil_platform::fs::only_user_or_root_can_change(path).unwrap_or(false);
    if !safe(&host) || !safe(dir) {
        // Windows: Zajil installed where other users may change files (a
        // second drive gives them that by default; a user's report on
        // Brave, #334). The browsers are given a copy in Zajil's own
        // folder, which only this user can change, kept the same as the
        // installed one, as for an AppImage.
        #[cfg(windows)]
        {
            eprintln!(
                "zajil: other users may change {}: the browsers get a copy of zajil-host",
                dir.display()
            );
            return zajil_platform::native_host::fixed_copy(&host, fixed).map_err(|err| {
                eprintln!("zajil: the browsers' copy of zajil-host: {err}");
                Problem::Unsafe
            });
        }
        #[cfg(not(windows))]
        return Err(Problem::Unsafe);
    }
    Ok(host)
}

/// Registers `zajil-host` with every supported browser for the current user
/// (ADR-0005), each allowing only Zajil's extension: its fixed id in
/// Chrome, Chromium and Edge, its Firefox id in Firefox. One registration
/// at a time in this program (the start and the settings button).
pub(crate) fn integrate_browsers(data: &Path) -> view::IntegrationView {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let appimage = std::env::var_os("APPIMAGE").is_some();
    let fixed = data.join(zajil_platform::native_host::FIXED_HOST_DIR);
    let host = std::env::current_exe()
        .map_err(|_| view::HostProblemView::Missing)
        .and_then(|exe| host_beside(&exe, appimage, &fixed));
    integrate_with(host, || SystemRegistrar::for_current_user(data))
}

/// [`integrate_browsers`] with the host found (or why not) and a way to
/// make the registrar, asked for only when there is a host.
fn integrate_with<G: NativeHostRegistrar>(
    host: Result<PathBuf, view::HostProblemView>,
    registrar: impl FnOnce() -> std::io::Result<G>,
) -> view::IntegrationView {
    let host = match host {
        Ok(host) => host,
        Err(problem) => {
            return view::IntegrationView {
                host_problem: Some(problem),
                browsers: Vec::new(),
            };
        }
    };
    match registrar() {
        Ok(registrar) => register_all(&registrar, &host),
        Err(err) => view::IntegrationView {
            host_problem: None,
            browsers: Browser::ALL
                .iter()
                .map(|b| view::BrowserIntegrationView {
                    browser: browser_name(*b).to_owned(),
                    error: Some(err.to_string()),
                })
                .collect(),
        },
    }
}

/// [`integrate_browsers`] with a given registrar and host.
fn register_all(registrar: &impl NativeHostRegistrar, host: &Path) -> view::IntegrationView {
    use zajil_platform::ids::{EXTENSION_ID_CHROMIUM, EXTENSION_ID_FIREFOX};
    let browsers = Browser::ALL
        .iter()
        .map(|&browser| {
            let allowed = if browser == Browser::Firefox {
                EXTENSION_ID_FIREFOX
            } else {
                EXTENSION_ID_CHROMIUM
            };
            view::BrowserIntegrationView {
                browser: browser_name(browser).to_owned(),
                error: registrar
                    .register(browser, host, &[allowed])
                    .err()
                    .map(|e| e.to_string()),
            }
        })
        .collect();
    view::IntegrationView {
        host_problem: None,
        browsers,
    }
}

fn browser_name(browser: Browser) -> &'static str {
    match browser {
        Browser::Chrome => "chrome",
        Browser::Chromium => "chromium",
        Browser::Edge => "edge",
        Browser::Brave => "brave",
        Browser::Opera => "opera",
        Browser::Vivaldi => "vivaldi",
        Browser::Firefox => "firefox",
    }
}

/// What went wrong, in words for the log.
pub(crate) fn integration_problems(integration: &view::IntegrationView) -> Vec<String> {
    use view::HostProblemView as Problem;
    match integration.host_problem {
        Some(Problem::Missing) => {
            return vec![
                "zajil-host is not next to the program, so no browser can reach Zajil".into(),
            ];
        }
        Some(Problem::Temporary) => {
            return vec![
                "the program runs from a place that goes away when it closes, so it registers nothing"
                    .into(),
            ];
        }
        Some(Problem::Unsafe) => {
            return vec![
                "zajil-host or its folder can be changed by another user, so it is not registered"
                    .into(),
            ];
        }
        None => {}
    }
    integration
        .browsers
        .iter()
        .filter_map(|b| {
            Some(format!(
                "cannot register with {}: {}",
                b.browser,
                b.error.as_ref()?
            ))
        })
        .collect()
}

/// "Fix the browser integration" (settings): registers again, and says
/// how it went for each browser.
#[tauri::command]
pub(crate) async fn fix_browser_integration<R: tauri::Runtime>(
    app: State<'_, App>,
    handle: tauri::AppHandle<R>,
) -> Result<view::IntegrationView, String> {
    if in_scene(&handle) {
        return Err("a ui-test scene registers nothing".into());
    }
    let (data, kept) = (app.data_dir.clone(), app.integration.clone());
    let integration = tokio::task::spawn_blocking(move || integrate_browsers(&data))
        .await
        .map_err(blocking_err)?;
    *kept.lock().unwrap_or_else(|e| e.into_inner()) = Some(integration.clone());
    Ok(integration)
}

/// How the last browser registration went (`null` while the one at the
/// start is still running, or in a ui-test scene).
#[tauri::command]
pub(crate) fn browser_integration(app: State<'_, App>) -> Option<view::IntegrationView> {
    app.integration
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M5: every browser is registered with `zajil-host`, each allowing
    /// only Zajil's extension (its fixed id; the Firefox id in Firefox),
    /// and nothing is left when unregistered.
    #[test]
    fn every_browser_is_registered_for_zajil_s_extension_only() {
        use zajil_platform::native_host::{Browser, NativeHostRegistrar, SystemRegistrar};
        let base = tempfile::tempdir().unwrap();
        let home = base.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        // A test registry root on Windows; only it, empty, may remain.
        let root = format!(r"Software\ZajilDesktopTest-{}", std::process::id());
        let registrar =
            SystemRegistrar::under(&home, &base.path().join("data/native-messaging"), &root);
        let host = base.path().join(if cfg!(windows) {
            "zajil-host.exe"
        } else {
            "zajil-host"
        });
        let integration = register_all(&registrar, &host);
        assert_eq!(integration.host_problem, None);
        assert_eq!(integration.browsers.len(), Browser::ALL.len());
        for (browser, view) in Browser::ALL.iter().zip(&integration.browsers) {
            assert_eq!(view.error, None, "{browser:?}");
            assert!(registrar.is_registered(*browser).unwrap());
            let manifest = std::fs::read_to_string(registrar.manifest_path(*browser)).unwrap();
            let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
            assert_eq!(manifest["path"], host.to_str().unwrap());
            let allowed = if *browser == Browser::Firefox {
                serde_json::json!({ "allowed_extensions": [zajil_platform::ids::EXTENSION_ID_FIREFOX] })
            } else {
                serde_json::json!({ "allowed_origins": [format!("chrome-extension://{}/", zajil_platform::ids::EXTENSION_ID_CHROMIUM)] })
            };
            let (key, value) = allowed.as_object().unwrap().iter().next().unwrap();
            assert_eq!(&manifest[key], value, "{browser:?}");
        }
        assert_eq!(integration_problems(&integration), Vec::<String>::new());
        for browser in Browser::ALL {
            registrar.unregister(browser).unwrap();
        }
        let mut left = Vec::new();
        for entry in walk(base.path()) {
            if entry.is_file() {
                left.push(entry);
            }
        }
        assert_eq!(left, Vec::<PathBuf>::new());
    }

    /// #129: from an AppImage, every browser is given the copy of
    /// `zajil-host` in Zajil's data folder, never the mount that goes away.
    #[cfg(unix)]
    #[test]
    fn from_an_appimage_browsers_are_given_the_lasting_copy() {
        use std::os::unix::fs::PermissionsExt;
        use zajil_platform::native_host::{Browser, FIXED_HOST_DIR};
        let base = tempfile::tempdir().unwrap();
        let mount = base.path().join("tmp/.mount_ZajilXy34/usr/bin");
        std::fs::create_dir_all(&mount).unwrap();
        let exe = mount.join("zajil-desktop");
        std::fs::write(&exe, "app").unwrap();
        let in_mount = mount.join("zajil-host");
        std::fs::write(&in_mount, "zajil-host").unwrap();
        std::fs::set_permissions(&in_mount, std::fs::Permissions::from_mode(0o755)).unwrap();
        let data = base.path().join("home/.local/share/zajil");
        let fixed = data.join(FIXED_HOST_DIR);

        let host = host_beside(&exe, true, &fixed);
        let registrar = || {
            SystemRegistrar::under(
                &base.path().join("home"),
                &data.join("native-messaging"),
                "Software\\Unused",
            )
        };
        let integration = integrate_with(host, || Ok(registrar()));
        let registrar = registrar();
        assert_eq!(integration.host_problem, None);
        let copy = fixed.join("zajil-host");
        for browser in Browser::ALL {
            let manifest = std::fs::read_to_string(registrar.manifest_path(browser)).unwrap();
            let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
            assert_eq!(manifest["path"], copy.to_str().unwrap(), "{browser:?}");
        }
        assert_eq!(std::fs::read(&copy).unwrap(), b"zajil-host");
        // The mount goes away when Zajil closes; the copy stays.
        std::fs::remove_dir_all(base.path().join("tmp")).unwrap();
        assert!(copy.is_file());
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            }
            out.push(path);
        }
        out
    }

    /// A host that is missing, not a program, or in a temporary place is
    /// not registered: the registrar is not even made, and the log says
    /// why. A host beside the program is found through a link to it.
    #[test]
    fn a_host_that_is_missing_or_unsafe_is_not_registered() {
        use view::HostProblemView as Problem;
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("app");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("zajil-desktop");
        std::fs::write(&exe, "app").unwrap();
        let fixed = base.path().join("data").join("bin");
        let host = dir.join(if cfg!(windows) {
            "zajil-host.exe"
        } else {
            "zajil-host"
        });
        let executable = |path: &Path, yes: bool| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if yes { 0o755 } else { 0o644 };
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
            }
            let _ = (path, yes);
        };
        executable(&dir, true);
        assert_eq!(
            host_beside(&exe, false, &fixed),
            Err(Problem::Missing),
            "no host"
        );
        std::fs::create_dir(&host).unwrap();
        assert_eq!(
            host_beside(&exe, false, &fixed),
            Err(Problem::Missing),
            "a folder"
        );
        std::fs::remove_dir(&host).unwrap();
        std::fs::write(&host, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            executable(&host, false);
            assert_eq!(
                host_beside(&exe, false, &fixed),
                Err(Problem::Missing),
                "not a program"
            );
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&host, std::fs::Permissions::from_mode(0o777)).unwrap();
            assert_eq!(
                host_beside(&exe, false, &fixed),
                Err(Problem::Unsafe),
                "writable by others"
            );
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
            std::fs::set_permissions(&host, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(
                host_beside(&exe, false, &fixed),
                Err(Problem::Unsafe),
                "folder writable by others"
            );
            executable(&dir, true);
        }
        executable(&host, true);
        let found = host_beside(&exe, false, &fixed).unwrap();
        #[cfg(unix)]
        assert_eq!(found, std::fs::canonicalize(&host).unwrap());
        #[cfg(windows)]
        assert_eq!(found, host);
        // Started through a link (a CLI or Homebrew link): still found.
        #[cfg(unix)]
        {
            let link = base.path().join("zajil");
            std::os::unix::fs::symlink(&exe, &link).unwrap();
            assert_eq!(host_beside(&link, false, &fixed), Ok(found.clone()));
        }
        // macOS moving the app aside: refused.
        let moved = base
            .path()
            .join("AppTranslocation/X/Zajil.app/Contents/MacOS");
        std::fs::create_dir_all(&moved).unwrap();
        std::fs::write(moved.join("zajil-desktop"), "app").unwrap();
        assert_eq!(
            host_beside(&moved.join("zajil-desktop"), false, &fixed),
            Err(Problem::Temporary)
        );
        // An AppImage's mount (#129): its host is copied to the fixed place,
        // and the copy is what browsers are given.
        let mount = base.path().join(".mount_ZajilAb12/usr/bin");
        std::fs::create_dir_all(&mount).unwrap();
        let in_mount = mount.join("zajil-desktop");
        std::fs::write(&in_mount, "app").unwrap();
        assert_eq!(
            host_beside(&in_mount, false, &fixed),
            Err(Problem::Missing),
            "no host in the AppImage"
        );
        let mounted_host = mount.join(host.file_name().unwrap());
        std::fs::write(&mounted_host, "zajil-host 1").unwrap();
        executable(&mounted_host, true);
        let copy = fixed.join(host.file_name().unwrap());
        assert_eq!(host_beside(&in_mount, false, &fixed), Ok(copy.clone()));
        assert_eq!(std::fs::read(&copy).unwrap(), b"zajil-host 1");
        // `APPIMAGE` set: the same, wherever it runs from.
        std::fs::write(&mounted_host, "zajil-host 2").unwrap();
        assert_eq!(host_beside(&in_mount, true, &fixed), Ok(copy.clone()));
        assert_eq!(std::fs::read(&copy).unwrap(), b"zajil-host 2");

        for problem in [Problem::Missing, Problem::Temporary, Problem::Unsafe] {
            let integration =
                integrate_with(Err(problem), || -> std::io::Result<SystemRegistrar> {
                    panic!("no registrar without a host")
                });
            assert_eq!(integration.host_problem, Some(problem));
            assert!(integration.browsers.is_empty());
            assert_eq!(integration_problems(&integration).len(), 1);
        }
    }
}
