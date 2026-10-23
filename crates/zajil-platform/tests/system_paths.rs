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
