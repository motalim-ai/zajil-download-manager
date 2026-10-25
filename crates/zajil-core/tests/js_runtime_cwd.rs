//! A relative `PATH` entry is never searched for a JavaScript runtime. Apart
//! from tests/js_runtime.rs: it changes the current folder, which the whole
//! test binary shares.

use std::path::{Path, PathBuf};

use zajil_core::video::find_js_runtime;

fn runtime(dir: &Path, name: &str, version: &str) -> PathBuf {
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let file = dir.join(format!("{name}{suffix}"));
    std::fs::copy(env!("CARGO_BIN_EXE_fake-js-runtime"), &file).unwrap();
    std::fs::write(dir.join(format!("{name}.version")), version).unwrap();
    file
}

#[test]
fn a_relative_path_entry_is_never_searched() {
    // A relative entry would find a `deno` in whatever folder Zajil runs.
    let dir = tempfile::tempdir().unwrap();
    runtime(dir.path(), "deno", "deno 2.5.0\n");
    let name = dir.path().file_name().unwrap();
    let parent = dir.path().parent().unwrap();
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(parent).unwrap();
    let found = find_js_runtime(Some(name), None, false);
    std::env::set_current_dir(previous).unwrap();
    assert_eq!(found, None);
}
