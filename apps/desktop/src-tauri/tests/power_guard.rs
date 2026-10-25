//! The owner's M3 rule, for the app too: tests never shut down or put to
//! sleep the machine they run on. The real `PowerControl` is named in one
//! place only, the schedule window's countdown (`src/schedule.rs`), and
//! never in its tests; CI also sets `ZAJIL_NO_POWER_ACTIONS` for the whole
//! workflow, which makes the real one refuse.

use std::fs;
use std::path::{Path, PathBuf};

/// Every `.rs` file of the app, as a path relative to `src-tauri`.
fn rust_files() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    let mut folders: Vec<PathBuf> = vec![root.join("src"), root.join("tests")];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                found.push((relative, fs::read_to_string(&path).unwrap()));
            }
        }
    }
    assert!(
        found.len() > 5,
        "the search found too few files: {}",
        found.len()
    );
    found
}

#[test]
fn the_real_power_control_is_only_used_by_the_schedule_window() {
    // Built from pieces, so this file does not match itself.
    let real = ["System", "Power"].concat();
    let mut seen = false;
    for (path, text) in rust_files() {
        if !text.contains(&real) {
            continue;
        }
        assert_eq!(path, "src/schedule.rs", "{path} names {real}");
        let (program, tests) = text
            .split_once("#[cfg(test)]")
            .expect("schedule.rs has its tests at the end");
        assert!(program.contains(&real));
        assert!(
            !tests.contains(&real),
            "the schedule window's tests name {real}"
        );
        seen = true;
    }
    assert!(
        seen,
        "the schedule window no longer names {real}: update this guard"
    );
}
