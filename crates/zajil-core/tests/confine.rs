//! Nothing can be written outside the download folder.

use std::fs;

use zajil_core::sanitize::{ConfineError, confine, sanitize_file_name};

/// The dangerous names listed in the M1 success criteria, plus a few more.
fn dangerous_names() -> Vec<String> {
    let absolute = std::env::temp_dir().join("zajil-abs-target");
    vec![
        "../../x".into(),
        "..\\..\\x".into(),
        absolute.to_string_lossy().into_owned(),
        "/etc/passwd".into(),
        "C:\\Windows\\System32\\drivers\\etc\\hosts".into(),
        "CON".into(),
        "NUL.txt".into(),
        "a\u{202E}txt.exe".into(),
        "n".repeat(10_000),
        "..".into(),
        "".into(),
    ]
}

#[test]
fn sanitized_dangerous_names_stay_in_the_folder() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("downloads");
    fs::create_dir(&dir).unwrap();

    for raw in dangerous_names() {
        let name = sanitize_file_name(&raw);
        let path = confine(&dir, &name).unwrap_or_else(|e| panic!("{raw:?}: {e}"));
        fs::write(&path, b"x").unwrap_or_else(|e| panic!("{raw:?} -> {name:?}: {e}"));
        assert_eq!(
            path.parent().unwrap().canonicalize().unwrap(),
            dir.canonicalize().unwrap()
        );
        assert!(!path.to_string_lossy().starts_with(r"\\?\"), "{path:?}");
    }

    // Only the download folder was written to.
    let outside: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(outside, vec!["downloads"]);
}

#[test]
fn confine_rejects_unsanitized_names() {
    let dir = tempfile::tempdir().unwrap();
    for raw in ["../x", "a/b", "a\\b", "..", ".", "", "/x"] {
        assert!(
            matches!(
                confine(dir.path(), raw),
                Err(ConfineError::OutsideFolder { .. })
            ),
            "{raw:?} was accepted"
        );
    }
    let absolute = dir.path().join("x");
    assert!(confine(dir.path(), absolute.to_str().unwrap()).is_err());
}

#[test]
fn confine_rejects_a_symbolic_link_to_outside() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("downloads");
    fs::create_dir(&dir).unwrap();
    let target = root.path().join("secret");
    fs::write(&target, b"keep").unwrap();

    let link = dir.join("file.txt");
    if !make_symlink(&target, &link) {
        // Creating symbolic links needs a privilege on some Windows setups.
        eprintln!("skipping: symbolic links are not permitted here");
        return;
    }
    assert!(matches!(
        confine(&dir, "file.txt"),
        Err(ConfineError::SymbolicLink { .. })
    ));
    assert_eq!(fs::read(&target).unwrap(), b"keep");
}

// Documented exception to "no cfg outside zajil-platform": test-only setup,
// because the standard library has no portable way to create a symlink.
fn make_symlink(target: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(target, link);
    made.is_ok()
}
