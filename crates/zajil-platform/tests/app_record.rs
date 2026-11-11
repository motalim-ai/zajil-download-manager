//! The record beside the browsers' copy of `zajil-host`: which Zajil it
//! starts when Zajil is not running, started only as it was recorded.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use zajil_platform::native_host::{APP_RECORD, FIXED_HOST_DIR, app_to_start, record_app};

/// A stand-in for Zajil's program, with `content`.
fn program(dir: &Path, content: &[u8]) -> PathBuf {
    let path = dir.join(if cfg!(windows) {
        "zajil-desktop.exe"
    } else {
        "zajil-desktop"
    });
    fs::write(&path, content).unwrap();
    path
}

#[test]
fn the_recorded_zajil_is_the_one_to_start() {
    let (install, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let app = program(install.path(), b"zajil 1.2.4");
    let dir = data.path().join(FIXED_HOST_DIR);

    record_app(&dir, &app).unwrap();
    let (path, _held) = app_to_start(&dir).unwrap();
    assert_eq!(path, app);
    assert!(zajil_platform::fs::only_user_or_root_can_change(&dir.join(APP_RECORD)).unwrap());

    // Recorded again (each start): one record, nothing else left.
    record_app(&dir, &app).unwrap();
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
}

#[test]
fn a_zajil_changed_since_it_was_recorded_is_not_started() {
    let (install, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let app = program(install.path(), b"zajil 1.2.4");
    let dir = data.path().join(FIXED_HOST_DIR);
    record_app(&dir, &app).unwrap();

    program(install.path(), b"something else");
    let err = app_to_start(&dir).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied, "{err}");
}

#[test]
fn without_a_record_there_is_nothing_to_start() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join(FIXED_HOST_DIR);
    zajil_platform::fs::private_dir(&dir).unwrap();
    assert_eq!(app_to_start(&dir).unwrap_err().kind(), ErrorKind::NotFound);
}

#[test]
fn a_record_that_is_not_two_lines_is_refused() {
    let (install, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let app = program(install.path(), b"zajil 1.2.4");
    let dir = data.path().join(FIXED_HOST_DIR);
    record_app(&dir, &app).unwrap();

    let record = dir.join(APP_RECORD);
    let text = fs::read_to_string(&record).unwrap();
    fs::write(&record, format!("{text}one more line\n")).unwrap();
    assert_eq!(
        app_to_start(&dir).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
}
