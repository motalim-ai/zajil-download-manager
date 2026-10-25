//! Which JavaScript runtime YouTube's challenges get (ADR-0009, owner's
//! order): Deno 2.3+ on the user's PATH, else Node 22+, else our
//! QuickJS-NG through `zajil-jsbox`, else none. A stand-in prints each
//! version (`fake-js-runtime`).

use std::path::{Path, PathBuf};

use zajil_core::video::{JsRuntime, find_js_runtime};

/// Puts a stand-in named `name` in `dir`, printing `version`.
fn runtime(dir: &Path, name: &str, version: &str) -> PathBuf {
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let file = dir.join(format!("{name}{suffix}"));
    std::fs::copy(env!("CARGO_BIN_EXE_fake-js-runtime"), &file).unwrap();
    std::fs::write(dir.join(format!("{name}.version")), version).unwrap();
    file
}

fn path_of(dirs: &[&Path]) -> std::ffi::OsString {
    std::env::join_paths(dirs).unwrap()
}

fn quickjs() -> Option<PathBuf> {
    Some("/app/zajil-jsbox".into())
}

#[test]
fn deno_first_then_node_then_quickjs() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let node = runtime(b.path(), "node", "v22.3.0\n");
    let path = path_of(&[a.path(), b.path()]);
    assert_eq!(
        find_js_runtime(Some(&path), quickjs(), false),
        Some(JsRuntime::Node(node.canonicalize().unwrap()))
    );
    let deno = runtime(a.path(), "deno", "deno 2.5.6 (stable, release)\nv8 14.0\n");
    assert_eq!(
        find_js_runtime(Some(&path), quickjs(), false),
        Some(JsRuntime::Deno(deno.canonicalize().unwrap()))
    );
    // Deno found after Node on the PATH still comes first.
    let path = path_of(&[b.path(), a.path()]);
    assert!(matches!(
        find_js_runtime(Some(&path), quickjs(), false),
        Some(JsRuntime::Deno(_))
    ));
}

#[test]
fn too_old_or_unreadable_versions_are_passed_over() {
    let dir = tempfile::tempdir().unwrap();
    runtime(dir.path(), "deno", "deno 2.2.9\n");
    runtime(dir.path(), "node", "v20.18.0\n");
    let path = path_of(&[dir.path()]);
    assert_eq!(
        find_js_runtime(Some(&path), quickjs(), false),
        Some(JsRuntime::QuickJs {
            jsbox: "/app/zajil-jsbox".into(),
        })
    );
    runtime(dir.path(), "node", "something else\n");
    assert!(matches!(
        find_js_runtime(Some(&path), quickjs(), false),
        Some(JsRuntime::QuickJs { .. })
    ));
    // Nothing at all: none, and yt-dlp gets none.
    assert_eq!(find_js_runtime(Some(&path), None, false), None);
    assert_eq!(find_js_runtime(None, None, false), None);
}

#[test]
fn always_quickjs_does_not_even_look_for_deno_or_node() {
    let dir = tempfile::tempdir().unwrap();
    runtime(dir.path(), "deno", "deno 2.5.0\n");
    let path = path_of(&[dir.path()]);
    assert!(matches!(
        find_js_runtime(Some(&path), quickjs(), true),
        Some(JsRuntime::QuickJs { .. })
    ));
    assert_eq!(find_js_runtime(Some(&path), None, true), None);
}

// Unix only: it changes mode bits. On Windows the same check reads the
// access list, tested in zajil-platform (`only_user_or_root_can_change`).
#[cfg(unix)]
#[test]
fn a_runtime_another_user_could_change_is_passed_over() {
    use std::os::unix::fs::PermissionsExt;
    // A folder anyone can write in.
    let open = tempfile::tempdir().unwrap();
    runtime(open.path(), "deno", "deno 2.5.0\n");
    std::fs::set_permissions(open.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
    let path = path_of(&[open.path()]);
    assert!(matches!(
        find_js_runtime(Some(&path), quickjs(), false),
        Some(JsRuntime::QuickJs { .. })
    ));
    // A private folder, but a file anyone can write.
    let private = tempfile::tempdir().unwrap();
    std::fs::set_permissions(private.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = runtime(private.path(), "node", "v22.3.0\n");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o777)).unwrap();
    let path = path_of(&[private.path()]);
    assert!(matches!(
        find_js_runtime(Some(&path), quickjs(), false),
        Some(JsRuntime::QuickJs { .. })
    ));
    // Fixed: it is used.
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        find_js_runtime(Some(&path), quickjs(), false),
        Some(JsRuntime::Node(_))
    ));
}

#[test]
fn a_runtime_behind_a_link_is_checked_and_named_at_its_real_place() {
    // As Homebrew, snap and Debian's alternatives install them.
    let (real, bin) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let node = runtime(real.path(), "node", "v22.3.0\n");
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let link = bin.path().join(format!("node{suffix}"));
    zajil_platform::fs::symlink_file(&node, &link)
        .expect("a test link must be made, never skipped");
    // The stand-in reads its version next to its real file.
    let path = path_of(&[bin.path()]);
    assert_eq!(
        find_js_runtime(Some(&path), quickjs(), false),
        Some(JsRuntime::Node(node.canonicalize().unwrap()))
    );
}

// Unix only, as above: a link whose real file sits in a folder anyone can
// write in.
#[cfg(unix)]
#[test]
fn a_link_to_a_runtime_another_user_could_change_is_passed_over() {
    use std::os::unix::fs::PermissionsExt;
    let (open, bin) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let deno = runtime(open.path(), "deno", "deno 2.5.0\n");
    std::fs::set_permissions(open.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
    zajil_platform::fs::symlink_file(&deno, &bin.path().join("deno"))
        .expect("a test link must be made");
    let path = path_of(&[bin.path()]);
    assert!(matches!(
        find_js_runtime(Some(&path), quickjs(), false),
        Some(JsRuntime::QuickJs { .. })
    ));
}
