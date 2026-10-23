//! Per-user folders: configuration, data, logs and the default download folder.
//!
//! Each OS has a pure `layout` function that turns OS inputs into
//! [`SystemPaths`]. The layouts compile everywhere so that their rules are
//! tested on every CI runner; only [`SystemPaths::new`] picks the one for the
//! current OS and reads the real inputs.

use std::path::{Path, PathBuf};

use crate::ids::APP_DIR_NAME;

/// Where Zajil keeps its files for the current user.
pub trait AppPaths {
    /// Settings (later: user rules, preferences).
    fn config_dir(&self) -> &Path;
    /// Application data (later: queues and history).
    fn data_dir(&self) -> &Path;
    /// Log files.
    fn log_dir(&self) -> &Path;
    /// Default folder for finished downloads. Shared with other apps, so it
    /// does not carry the `zajil` name.
    fn download_dir(&self) -> &Path;
}

/// Errors while resolving [`SystemPaths`].
#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    #[error("could not find the home folder of the current user")]
    NoHome,
    #[error("the system returned a relative path for {what}: {path}")]
    NotAbsolute { what: &'static str, path: PathBuf },
}

/// The folders of the current user on the current OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemPaths {
    config_dir: PathBuf,
    data_dir: PathBuf,
    log_dir: PathBuf,
    download_dir: PathBuf,
}

impl SystemPaths {
    /// Resolves the folders for the current user. Nothing is created on disk.
    pub fn new() -> Result<Self, PathsError> {
        let paths = resolve()?;
        for (what, path) in [
            ("the configuration folder", &paths.config_dir),
            ("the data folder", &paths.data_dir),
            ("the log folder", &paths.log_dir),
            ("the download folder", &paths.download_dir),
        ] {
            if !path.is_absolute() {
                return Err(PathsError::NotAbsolute {
                    what,
                    path: path.clone(),
                });
            }
        }
        Ok(paths)
    }
}

impl AppPaths for SystemPaths {
    fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    fn download_dir(&self) -> &Path {
        &self.download_dir
    }
}

#[cfg(target_os = "windows")]
fn resolve() -> Result<SystemPaths, PathsError> {
    let roaming = dirs::config_dir().ok_or(PathsError::NoHome)?;
    let local = dirs::data_local_dir().ok_or(PathsError::NoHome)?;
    let home = dirs::home_dir().ok_or(PathsError::NoHome)?;
    let downloads = dirs::download_dir().unwrap_or_else(|| home.join("Downloads"));
    Ok(windows::layout(&roaming, &local, &downloads))
}

#[cfg(target_os = "macos")]
fn resolve() -> Result<SystemPaths, PathsError> {
    let home = dirs::home_dir().ok_or(PathsError::NoHome)?;
    Ok(macos::layout(&home))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn resolve() -> Result<SystemPaths, PathsError> {
    let home = dirs::home_dir().ok_or(PathsError::NoHome)?;
    // `dirs` reads `XDG_DOWNLOAD_DIR` from `user-dirs.dirs`.
    let user_downloads = dirs::download_dir();
    Ok(linux::layout(
        &home,
        |key| std::env::var_os(key),
        user_downloads,
    ))
}

/// Windows: roaming settings, local data and logs, and the Downloads known
/// folder.
pub mod windows {
    use super::*;

    /// `roaming` is `%APPDATA%`, `local` is `%LOCALAPPDATA%` and `downloads`
    /// is the Downloads known folder.
    pub fn layout(roaming: &Path, local: &Path, downloads: &Path) -> SystemPaths {
        let data_dir = local.join(APP_DIR_NAME);
        SystemPaths {
            config_dir: roaming.join(APP_DIR_NAME),
            log_dir: data_dir.join("logs"),
            data_dir,
            download_dir: downloads.to_path_buf(),
        }
    }
}

/// macOS: `~/Library/Application Support`, `~/Library/Logs` and
/// `~/Downloads`.
pub mod macos {
    use super::*;

    pub fn layout(home: &Path) -> SystemPaths {
        let support = home
            .join("Library")
            .join("Application Support")
            .join(APP_DIR_NAME);
        SystemPaths {
            config_dir: support.clone(),
            data_dir: support,
            log_dir: home.join("Library").join("Logs").join(APP_DIR_NAME),
            download_dir: home.join("Downloads"),
        }
    }
}

/// Linux and other Unix systems: the XDG Base Directory specification.
pub mod linux {
    use std::ffi::OsString;

    use super::*;

    /// `env` looks up environment variables. `user_downloads` is
    /// `XDG_DOWNLOAD_DIR` from `user-dirs.dirs`, if set.
    pub fn layout(
        home: &Path,
        env: impl Fn(&str) -> Option<OsString>,
        user_downloads: Option<PathBuf>,
    ) -> SystemPaths {
        // XDG: an unset, empty or relative value means "use the default".
        let base = |key: &str, default: &[&str]| -> PathBuf {
            env(key)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .unwrap_or_else(|| default.iter().fold(home.to_path_buf(), |p, c| p.join(c)))
        };
        SystemPaths {
            config_dir: base("XDG_CONFIG_HOME", &[".config"]).join(APP_DIR_NAME),
            data_dir: base("XDG_DATA_HOME", &[".local", "share"]).join(APP_DIR_NAME),
            log_dir: base("XDG_STATE_HOME", &[".local", "state"])
                .join(APP_DIR_NAME)
                .join("logs"),
            download_dir: user_downloads
                .filter(|path| path.is_absolute())
                .unwrap_or_else(|| home.join("Downloads")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::OsString;

    use super::*;

    fn home() -> PathBuf {
        // An absolute path on every OS the tests run on.
        std::env::temp_dir().join("home")
    }

    fn env_from(pairs: &[(&str, PathBuf)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone().into_os_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn linux_defaults_follow_xdg() {
        let home = home();
        let paths = linux::layout(&home, env_from(&[]), None);
        assert_eq!(paths.config_dir(), home.join(".config").join("zajil"));
        assert_eq!(
            paths.data_dir(),
            home.join(".local").join("share").join("zajil")
        );
        assert_eq!(
            paths.log_dir(),
            home.join(".local").join("state").join("zajil").join("logs")
        );
        assert_eq!(paths.download_dir(), home.join("Downloads"));
    }

    #[test]
    fn linux_uses_absolute_xdg_variables() {
        let home = home();
        let custom = std::env::temp_dir().join("custom");
        let paths = linux::layout(
            &home,
            env_from(&[
                ("XDG_CONFIG_HOME", custom.join("cfg")),
                ("XDG_DATA_HOME", custom.join("data")),
                ("XDG_STATE_HOME", custom.join("state")),
            ]),
            Some(custom.join("dl")),
        );
        assert_eq!(paths.config_dir(), custom.join("cfg").join("zajil"));
        assert_eq!(paths.data_dir(), custom.join("data").join("zajil"));
        assert_eq!(
            paths.log_dir(),
            custom.join("state").join("zajil").join("logs")
        );
        assert_eq!(paths.download_dir(), custom.join("dl"));
    }

    #[test]
    fn linux_ignores_empty_and_relative_xdg_variables() {
        let home = home();
        let paths = linux::layout(
            &home,
            env_from(&[
                ("XDG_CONFIG_HOME", PathBuf::new()),
                ("XDG_DATA_HOME", PathBuf::from("relative/data")),
            ]),
            Some(PathBuf::from("relative/dl")),
        );
        assert_eq!(paths.config_dir(), home.join(".config").join("zajil"));
        assert_eq!(
            paths.data_dir(),
            home.join(".local").join("share").join("zajil")
        );
        assert_eq!(paths.download_dir(), home.join("Downloads"));
    }

    #[test]
    fn macos_layout_uses_library_folders() {
        let home = home();
        let paths = macos::layout(&home);
        let support = home
            .join("Library")
            .join("Application Support")
            .join("zajil");
        assert_eq!(paths.config_dir(), support);
        assert_eq!(paths.data_dir(), support);
        assert_eq!(
            paths.log_dir(),
            home.join("Library").join("Logs").join("zajil")
        );
        assert_eq!(paths.download_dir(), home.join("Downloads"));
    }

    #[test]
    fn windows_layout_splits_roaming_and_local() {
        let roaming = PathBuf::from(r"C:\Users\u\AppData\Roaming");
        let local = PathBuf::from(r"C:\Users\u\AppData\Local");
        let downloads = PathBuf::from(r"D:\Downloads");
        let paths = windows::layout(&roaming, &local, &downloads);
        assert_eq!(paths.config_dir(), roaming.join("zajil"));
        assert_eq!(paths.data_dir(), local.join("zajil"));
        assert_eq!(paths.log_dir(), local.join("zajil").join("logs"));
        assert_eq!(paths.download_dir(), downloads);
    }
}
