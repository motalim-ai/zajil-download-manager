//! Resolves the real folders on the OS running the test.

use std::path::Path;

use zajil_platform::{AppPaths, SystemPaths};

fn contains_app_dir(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str() == zajil_platform::ids::APP_DIR_NAME)
}

#[test]
fn resolves_absolute_folders_for_the_current_user() {
    let paths = SystemPaths::new().expect("folders resolve on CI runners");
    for dir in [
        paths.config_dir(),
        paths.data_dir(),
        paths.log_dir(),
        paths.download_dir(),
    ] {
        assert!(dir.is_absolute(), "{} is not absolute", dir.display());
    }
    assert!(contains_app_dir(paths.config_dir()));
    assert!(contains_app_dir(paths.data_dir()));
    assert!(contains_app_dir(paths.log_dir()));
}

#[test]
fn resolving_is_stable() {
    assert_eq!(SystemPaths::new().unwrap(), SystemPaths::new().unwrap());
}

// Each system checks that its own implementation was chosen and follows its
// own rules (ADR-0006, rule 3). `cfg` is allowed here: this is
// `zajil-platform`'s own test.

#[cfg(target_os = "windows")]
#[test]
fn windows_uses_roaming_and_local_app_data_and_the_downloads_known_folder() {
    let paths = SystemPaths::new().unwrap();
    let roaming = std::path::PathBuf::from(std::env::var_os("APPDATA").expect("APPDATA"));
    let local = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA"));
    assert_eq!(paths.config_dir(), roaming.join("zajil"));
    assert_eq!(paths.data_dir(), local.join("zajil"));
    assert_eq!(paths.log_dir(), local.join("zajil").join("logs"));
    assert!(
        paths.download_dir().ends_with("Downloads"),
        "{}",
        paths.download_dir().display()
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_uses_the_library_folders_of_the_home_folder() {
    let paths = SystemPaths::new().unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME"));
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

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn linux_follows_the_xdg_variables_or_their_defaults() {
    let paths = SystemPaths::new().unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    let base = |var: &str, default: &[&str]| {
        std::env::var_os(var)
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| default.iter().fold(home.clone(), |p, c| p.join(c)))
    };
    assert_eq!(
        paths.config_dir(),
        base("XDG_CONFIG_HOME", &[".config"]).join("zajil")
    );
    assert_eq!(
        paths.data_dir(),
        base("XDG_DATA_HOME", &[".local", "share"]).join("zajil")
    );
    assert_eq!(
        paths.log_dir(),
        base("XDG_STATE_HOME", &[".local", "state"])
            .join("zajil")
            .join("logs")
    );
}
