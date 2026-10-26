//! The fixed copy of `zajil-host` (#129): when Zajil runs from an AppImage,
//! whose folder goes away when it closes, browsers are given a copy in
//! Zajil's data folder, kept the same as the AppImage's, and never written
//! through a link.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use zajil_platform::native_host::{FIXED_HOST_DIR, fixed_copy};

/// A stand-in for the AppImage's `zajil-host`, with `content`.
fn host(dir: &Path, content: &[u8]) -> PathBuf {
    let path = dir.join("zajil-host");
    fs::write(&path, content).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn the_host_is_copied_once_into_a_folder_only_its_user_can_change() {
    let (mount, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let source = host(mount.path(), b"zajil-host 1");
    let dir = data.path().join(FIXED_HOST_DIR);

    let copy = fixed_copy(&source, &dir).unwrap();
    assert_eq!(copy, dir.join("zajil-host"));
    assert_eq!(fs::read(&copy).unwrap(), b"zajil-host 1");
    let meta = fs::symlink_metadata(&copy).unwrap();
    assert!(meta.is_file());
    assert_eq!(meta.permissions().mode() & 0o777, 0o700);
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(zajil_platform::fs::only_user_or_root_can_change(&copy).unwrap());
    assert!(zajil_platform::fs::only_user_or_root_can_change(&dir).unwrap());

    // The same version again: the copy is kept, not written again.
    let again = fixed_copy(&source, &dir).unwrap();
    assert_eq!(again, copy);
    assert_eq!(fs::metadata(&copy).unwrap().ino(), meta.ino());
    // Nothing else is left in the folder.
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
}

#[test]
fn an_updated_appimage_replaces_the_copy() {
    let (mount, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let dir = data.path().join(FIXED_HOST_DIR);
    fixed_copy(&host(mount.path(), b"zajil-host 1"), &dir).unwrap();

    let copy = fixed_copy(&host(mount.path(), b"zajil-host 2, longer"), &dir).unwrap();
    assert_eq!(fs::read(&copy).unwrap(), b"zajil-host 2, longer");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
}

/// A link, or a second name of a file elsewhere, in the copy's place is
/// replaced by the copy; the file it leads to is never written.
#[test]
fn a_link_in_the_copy_s_place_is_replaced_and_what_it_leads_to_untouched() {
    for hard in [false, true] {
        let (mount, data, elsewhere) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let dir = data.path().join(FIXED_HOST_DIR);
        zajil_platform::fs::private_dir(&dir).unwrap();
        let outside = elsewhere.path().join("other-program");
        fs::write(&outside, b"someone else's").unwrap();
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o700)).unwrap();
        if hard {
            fs::hard_link(&outside, dir.join("zajil-host")).unwrap();
        } else {
            std::os::unix::fs::symlink(&outside, dir.join("zajil-host")).unwrap();
        }

        let copy = fixed_copy(&host(mount.path(), b"zajil-host 1"), &dir).unwrap();
        assert_eq!(
            fs::read(&outside).unwrap(),
            b"someone else's",
            "hard: {hard}"
        );
        let meta = fs::symlink_metadata(&copy).unwrap();
        assert!(meta.is_file() && meta.nlink() == 1, "hard: {hard}");
        assert_eq!(fs::read(&copy).unwrap(), b"zajil-host 1");
    }
}

/// A folder other users can change is refused: nothing is copied into it.
#[test]
fn a_folder_others_can_change_is_refused() {
    let (mount, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let dir = data.path().join(FIXED_HOST_DIR);
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).unwrap();

    assert!(fixed_copy(&host(mount.path(), b"zajil-host 1"), &dir).is_err());
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
}
