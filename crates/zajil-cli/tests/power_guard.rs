//! The check the owner asked for (M3 plan): tests never shut down or put
//! to sleep the machine they run on. Tests use a mock `PowerControl`; this
//! guard makes sure nothing else can slip in:
//! 1. the real implementation is named only where the program uses it:
//!    its definition in `zajil-platform` and `zajil queue run`; never in a
//!    test file;
//! 2. every test starts `zajil` through `tests/common/mod.rs`, which sets
//!    `ZAJIL_NO_POWER_ACTIONS` (the real implementation then refuses);
//! 3. CI sets `ZAJIL_NO_POWER_ACTIONS` for the whole workflow.

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `.rs` file under `crates/`, as a path relative to the root.
fn rust_files() -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut folders = vec![root().join("crates")];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let relative = path
                    .strip_prefix(root())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                found.push((relative, fs::read_to_string(&path).unwrap()));
            }
        }
    }
    assert!(
        found.len() > 20,
        "the search found too few files: {}",
        found.len()
    );
    found
}

/// The guard's own words are built from pieces, so this file does not
/// match itself.
fn words(pieces: &[&str]) -> String {
    pieces.concat()
}

#[test]
fn the_real_power_control_is_only_used_by_the_program() {
    let real = words(&["System", "Power"]);
    let allowed = [
        "crates/zajil-platform/src/power.rs",
        "crates/zajil-cli/src/queue.rs",
    ];
    for (path, text) in rust_files() {
        if text.contains(&real) {
            assert!(allowed.contains(&path.as_str()), "{path} names {real}");
        }
    }
    // And the program's use is not inside a test module.
    let (_, cli) = rust_files()
        .into_iter()
        .find(|(path, _)| path == "crates/zajil-cli/src/queue.rs")
        .unwrap();
    assert!(!cli.contains("#[cfg(test)]"), "queue.rs grew a test module");
}

#[test]
fn every_test_starts_zajil_with_power_actions_forbidden() {
    let binary = words(&["CARGO_BIN_EXE", "_zajil"]);
    let users: Vec<String> = rust_files()
        .into_iter()
        .filter(|(_, text)| text.contains(&binary))
        .map(|(path, _)| path)
        .collect();
    assert_eq!(users, ["crates/zajil-cli/tests/common/mod.rs"]);
    let common = fs::read_to_string(root().join("crates/zajil-cli/tests/common/mod.rs")).unwrap();
    assert!(
        common.contains(".env(zajil_platform::power::NO_POWER_ACTIONS"),
        "{common}"
    );
    assert_eq!(
        zajil_platform::power::NO_POWER_ACTIONS,
        "ZAJIL_NO_POWER_ACTIONS"
    );
}

#[test]
fn ci_forbids_power_actions_for_the_whole_workflow() {
    // Windows checks files out with CRLF line ends.
    let ci = fs::read_to_string(root().join(".github/workflows/ci.yml"))
        .unwrap()
        .replace("\r\n", "\n");
    let env = ci
        .split("\nenv:\n")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .expect("a top-level env block");
    assert!(env.contains("ZAJIL_NO_POWER_ACTIONS: \"1\""), "{env}");
}
