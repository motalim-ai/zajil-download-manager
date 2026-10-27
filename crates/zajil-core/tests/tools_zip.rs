//! A tool released as a zip (yt-dlp's "onedir" build on macOS and Windows,
//! owner's decision, 2026-10-27): kept only if the zip is the pinned one
//! and what it holds has the pinned tree SHA-256, unpacked once, and
//! checked whole before every run. Every file is served by the local test
//! server; nothing here reaches the internet.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use zajil_core::tools::{Archive, Pinned, Tool, ToolError, Toolbox, tree_sha256, unpack_zip};
use zajil_core::{Engine, Options, Stop};
use zajil_testserver::{FileSpec, TestServer, Tls, sha256_hex, test_ca_der};

const VERSION: &str = "2099.01.01";
const PROGRAM: &str = "yt-dlp_test";

fn trusting_engine() -> Engine {
    Engine::with_test_root(Options::default(), &test_ca_der()).unwrap()
}

fn https_server() -> TestServer {
    TestServer::https(Tls::trusted(&["localhost", "127.0.0.1"]), "localhost")
}

/// A zip of `entries`: `(name, bytes, unix mode)`; a name ending in `/` is
/// a folder, and a mode of a link (`0o120…`) makes a link to `bytes`.
fn zip_of(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut out);
        for (name, bytes, mode) in entries {
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(*mode);
            if name.ends_with('/') {
                zip.add_directory(*name, options).unwrap();
            } else if mode & 0o170_000 == 0o120_000 {
                // A link, to what `bytes` says.
                zip.add_symlink(*name, std::str::from_utf8(bytes).unwrap(), options)
                    .unwrap();
            } else {
                zip.start_file(*name, options).unwrap();
                zip.write_all(bytes).unwrap();
            }
        }
        zip.finish().unwrap();
    }
    out.into_inner()
}

/// The files of the test tool: a program at the top and its folder.
fn good_entries() -> Vec<(&'static str, &'static [u8], u32)> {
    vec![
        ("_internal/", b"", 0o755),
        ("_internal/lib/", b"", 0o755),
        ("_internal/lib/one.so", b"first library", 0o755),
        ("_internal/base_library.zip", b"python modules", 0o644),
        (PROGRAM, b"the program", 0o755),
    ]
}

/// The tree SHA-256 of `entries`' files, as Zajil pins it.
fn tree_of(entries: &[(&str, &[u8], u32)]) -> String {
    let files: Vec<(String, String)> = entries
        .iter()
        .filter(|(name, _, _)| !name.ends_with('/'))
        .map(|(name, bytes, _)| ((*name).to_owned(), sha256_hex(bytes)))
        .collect();
    tree_sha256(&files)
}

/// A toolbox in `root` whose yt-dlp is the zip at `url`.
fn toolbox(root: &Path, url: String, sha256: String, tree: String) -> Toolbox {
    Toolbox::with_pins(
        root,
        Box::new(move |tool, _| {
            (tool == Tool::YtDlp).then(|| Pinned {
                version: VERSION.to_owned(),
                url: url.clone(),
                sha256: sha256.clone(),
                archive: Some(Archive {
                    folder: "yt-dlp_test".to_owned(),
                    program: PROGRAM.to_owned(),
                    tree_sha256: tree.clone(),
                }),
            })
        }),
    )
}

/// Everything under `root`, as paths relative to it.
fn tree(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            out.push(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
            if path.is_dir() {
                walk(root, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

async fn install(toolbox: &Toolbox) -> Result<PathBuf, ToolError> {
    toolbox
        .install(&trusting_engine(), Tool::YtDlp, &Stop::new(), &mut |_| {})
        .await
}

/// A served zip of `entries`, and a toolbox in a new folder pinned to it
/// (with `tree`, or the right tree).
fn served(
    entries: &[(&str, &[u8], u32)],
    tree: Option<String>,
) -> (TestServer, tempfile::TempDir, Toolbox) {
    let server = https_server();
    let bytes = zip_of(entries);
    server.put("/yt-dlp.zip", FileSpec::new(bytes.clone()));
    let root = tempfile::tempdir().unwrap();
    let tools = toolbox(
        &root.path().join("tools"),
        server.url("/yt-dlp.zip"),
        sha256_hex(&bytes),
        tree.unwrap_or_else(|| tree_of(entries)),
    );
    (server, root, tools)
}

#[tokio::test]
async fn a_pinned_zip_is_unpacked_once_and_checked_whole_before_use() {
    let entries = good_entries();
    let (_server, root, tools) = served(&entries, None);
    // An earlier one-file build of the same version, agreed to before.
    let old = root.path().join("tools/yt-dlp").join(VERSION);
    zajil_platform::fs::private_dir(&old).unwrap();
    zajil_platform::fs::create_private(&old.join("yt-dlp"))
        .unwrap()
        .write_all(b"old")
        .unwrap();
    let older = root.path().join("tools/yt-dlp/2098.01.01");
    zajil_platform::fs::private_dir(&older).unwrap();
    assert!(tools.has_an_earlier_one(Tool::YtDlp));

    let path = install(&tools).await.unwrap();

    assert_eq!(
        path,
        root.path()
            .join("tools/yt-dlp")
            .join(VERSION)
            .join("yt-dlp_test")
            .join(PROGRAM)
    );
    assert_eq!(fs::read(&path).unwrap(), b"the program");
    assert_eq!(tools.verified(Tool::YtDlp).await.unwrap().path(), path);
    assert!(!tools.has_an_earlier_one(Tool::YtDlp));
    // The earlier builds are gone, and nothing but the tool is left.
    assert_eq!(
        tree(&root.path().join("tools")),
        [
            "yt-dlp",
            "yt-dlp/2099.01.01",
            "yt-dlp/2099.01.01/yt-dlp_test",
            "yt-dlp/2099.01.01/yt-dlp_test/_internal",
            "yt-dlp/2099.01.01/yt-dlp_test/_internal/base_library.zip",
            "yt-dlp/2099.01.01/yt-dlp_test/_internal/lib",
            "yt-dlp/2099.01.01/yt-dlp_test/_internal/lib/one.so",
            "yt-dlp/2099.01.01/yt-dlp_test/yt-dlp_test",
        ]
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let unpacked = path.parent().unwrap();
        assert_eq!(mode(&path), 0o700);
        assert_eq!(mode(&unpacked.join("_internal/lib/one.so")), 0o700);
        assert_eq!(mode(&unpacked.join("_internal/base_library.zip")), 0o600);
        assert_eq!(mode(&unpacked.join("_internal/lib")), 0o700);
    }
}

/// Only another version or build counts as "earlier" (brought up to date
/// without asking): not an empty folder a failed install left, nor the
/// pinned one changed (that one is refused before it runs). Told without
/// hashing anything (review of #278).
#[tokio::test]
async fn only_another_version_or_build_is_an_earlier_one() {
    let entries = good_entries();
    let (_server, root, tools) = served(&entries, None);
    let version = root.path().join("tools/yt-dlp").join(VERSION);
    assert!(!tools.has_an_earlier_one(Tool::YtDlp));
    // A failed install's empty folders.
    zajil_platform::fs::private_dir(&version.join("yt-dlp_test/_internal")).unwrap();
    zajil_platform::fs::private_dir(&root.path().join("tools/yt-dlp/2098.01.01")).unwrap();
    assert!(!tools.has_an_earlier_one(Tool::YtDlp));
    // Another version's program.
    zajil_platform::fs::create_private(&root.path().join("tools/yt-dlp/2098.01.01/yt-dlp"))
        .unwrap();
    assert!(tools.has_an_earlier_one(Tool::YtDlp));
    fs::remove_dir_all(root.path().join("tools/yt-dlp/2098.01.01")).unwrap();
    // The pinned one, installed and then changed: not "earlier".
    fs::remove_dir_all(version.join("yt-dlp_test")).unwrap();
    let path = install(&tools).await.unwrap();
    fs::write(&path, b"changed").unwrap();
    assert!(!tools.has_an_earlier_one(Tool::YtDlp));
    assert!(tools.verified(Tool::YtDlp).await.is_err());
}

#[tokio::test]
async fn a_file_changed_added_or_missing_after_the_install_stops_the_tool() {
    let entries = good_entries();
    let (_server, _root, tools) = served(&entries, None);
    let path = install(&tools).await.unwrap();
    let unpacked = path.parent().unwrap().to_path_buf();
    let library = unpacked.join("_internal/lib/one.so");

    // Changed.
    fs::write(&library, b"another library").unwrap();
    assert!(matches!(
        tools.verified(Tool::YtDlp).await,
        Err(ToolError::Mismatch { .. })
    ));
    fs::write(&library, b"first library").unwrap();
    assert!(tools.verified(Tool::YtDlp).await.is_ok());

    // Added (Python would load it).
    let extra = unpacked.join("_internal/sitecustomize.py");
    zajil_platform::fs::create_private(&extra)
        .unwrap()
        .write_all(b"import os")
        .unwrap();
    assert!(matches!(
        tools.verified(Tool::YtDlp).await,
        Err(ToolError::Mismatch { .. })
    ));
    fs::remove_file(&extra).unwrap();
    assert!(tools.verified(Tool::YtDlp).await.is_ok());

    // Missing.
    fs::remove_file(&library).unwrap();
    assert!(matches!(
        tools.verified(Tool::YtDlp).await,
        Err(ToolError::Mismatch { .. })
    ));
    assert!(!tools.is_installed(Tool::YtDlp));
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_or_a_folder_others_can_change_inside_stops_the_tool() {
    use std::os::unix::fs::PermissionsExt;
    let entries = good_entries();
    let (_server, _root, tools) = served(&entries, None);
    let path = install(&tools).await.unwrap();
    let unpacked = path.parent().unwrap().to_path_buf();

    let lib = unpacked.join("_internal/lib");
    fs::set_permissions(&lib, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        tools.verified(Tool::YtDlp).await,
        Err(ToolError::NotPrivate(_))
    ));
    fs::set_permissions(&lib, fs::Permissions::from_mode(0o700)).unwrap();

    let library = lib.join("one.so");
    let elsewhere = tempfile::tempdir().unwrap();
    let target = elsewhere.path().join("one.so");
    fs::copy(&library, &target).unwrap();
    fs::remove_file(&library).unwrap();
    std::os::unix::fs::symlink(&target, &library).unwrap();
    assert!(tools.verified(Tool::YtDlp).await.is_err());
}

#[tokio::test]
async fn a_zip_that_is_not_the_pinned_one_is_never_unpacked() {
    let entries = good_entries();
    let server = https_server();
    server.put("/yt-dlp.zip", FileSpec::new(zip_of(&entries)));
    let root = tempfile::tempdir().unwrap();
    let tools = toolbox(
        &root.path().join("tools"),
        server.url("/yt-dlp.zip"),
        "0".repeat(64),
        tree_of(&entries),
    );
    assert!(matches!(
        install(&tools).await,
        Err(ToolError::Mismatch { .. })
    ));
    // Nothing unpacked anywhere, and nothing left.
    assert_eq!(
        tree(&root.path().join("tools")),
        ["yt-dlp", "yt-dlp/2099.01.01"]
    );
}

#[tokio::test]
async fn a_pinned_zip_whose_files_are_not_the_pinned_ones_is_refused_and_removed() {
    let entries = good_entries();
    let (_server, root, tools) = served(&entries, Some("0".repeat(64)));
    assert!(matches!(
        install(&tools).await,
        Err(ToolError::Mismatch { .. })
    ));
    assert_eq!(
        tree(&root.path().join("tools")),
        ["yt-dlp", "yt-dlp/2099.01.01"]
    );
    assert!(!tools.is_installed(Tool::YtDlp));
}

#[test]
fn a_zip_that_could_write_outside_its_folder_or_hold_a_link_is_refused() {
    let program: (&str, &[u8], u32) = (PROGRAM, b"the program", 0o755);
    for (why, bad) in [
        ("up", ("../outside", &b"x"[..], 0o644)),
        ("up inside", ("_internal/../../outside", &b"x"[..], 0o644)),
        ("absolute", ("/etc/outside", &b"x"[..], 0o644)),
        ("backslash", ("_internal\\..\\outside", &b"x"[..], 0o644)),
        ("drive", ("C:outside", &b"x"[..], 0o644)),
        ("a link", ("_internal/link", &b"/etc/passwd"[..], 0o120_777)),
        ("trailing dot", ("_internal/name.", &b"x"[..], 0o644)),
        ("a Windows device", ("_internal/NUL.dll", &b"x"[..], 0o644)),
        ("a Windows port", ("_internal/com1", &b"x"[..], 0o644)),
    ] {
        let bytes = zip_of(&[program, bad]);
        let dir = tempfile::tempdir().unwrap();
        let into = dir.path().join("tool");
        fs::write(dir.path().join("t.zip"), &bytes).unwrap();
        let zip = fs::File::open(dir.path().join("t.zip")).unwrap();
        assert!(unpack_zip(zip, &into, PROGRAM).is_err(), "{why}");
        assert!(!dir.path().join("outside").exists(), "{why}");
    }
    // No program at its top.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("t.zip"),
        zip_of(&[("_internal/x", b"x", 0o644)]),
    )
    .unwrap();
    let zip = fs::File::open(dir.path().join("t.zip")).unwrap();
    assert!(unpack_zip(zip, &dir.path().join("tool"), PROGRAM).is_err());
    // A good one.
    fs::write(dir.path().join("good.zip"), zip_of(&good_entries())).unwrap();
    let zip = fs::File::open(dir.path().join("good.zip")).unwrap();
    assert_eq!(
        unpack_zip(zip, &dir.path().join("good"), PROGRAM).unwrap(),
        tree_of(&good_entries())
    );
}

/// The tree SHA-256 is the pins' fixed form: sorted paths, then each
/// file's SHA-256, whatever order the zip holds them in.
#[test]
fn the_tree_sha256_is_of_the_sorted_paths_and_files() {
    let a = vec![
        ("b".to_owned(), "1".repeat(64)),
        ("a/c".to_owned(), "2".repeat(64)),
    ];
    let b = vec![a[1].clone(), a[0].clone()];
    assert_eq!(tree_sha256(&a), tree_sha256(&b));
    let text = format!("a/c\0{}\nb\0{}\n", "2".repeat(64), "1".repeat(64));
    assert_eq!(tree_sha256(&a), sha256_hex(text.as_bytes()));
    assert_ne!(tree_sha256(&a), tree_sha256(&a[..1]));
}
