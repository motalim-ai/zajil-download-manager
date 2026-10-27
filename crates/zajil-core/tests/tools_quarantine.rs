//! macOS: a Zajil started by a marked process is marked too (Chrome marks
//! its own process, and the system marks every program it starts the same
//! way), and the system then marks every file it, or a program it starts,
//! writes (`com.apple.quarantine`). Gatekeeper refused yt-dlp's
//! Python.framework so marked as "damaged" (owner's report before 1.0).
//! Zajil removes the mark from each tool it checked to be the pinned one,
//! and from the folders that hold it: when it installs it, and before each
//! run (a tool unpacked before this fix).
//!
//! This test's process is marked as Chrome marks itself. The tools are
//! served by the local test server; nothing here reaches the internet.
#![cfg(target_os = "macos")]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use zajil_core::tools::{Archive, Pinned, Tool, Toolbox, tree_sha256};
use zajil_core::video::{Runs, VideoRunner};
use zajil_core::{Engine, Options, Stop};
use zajil_platform::origin;
use zajil_testserver::{FileSpec, TestServer, Tls, random_bytes, sha256_hex, test_ca_der};

/// Marks this process, as Chrome marks itself; whether the files it writes
/// now carry the mark (a file system that cannot hold it says no).
fn marked_as_chrome(dir: &Path) -> bool {
    origin::quarantine_this_process("com.google.Chrome").unwrap();
    let probe = dir.join("probe");
    fs::write(&probe, b"").unwrap();
    let marks = origin::read(&probe).unwrap().is_some();
    fs::remove_file(&probe).unwrap();
    if !marks {
        assert!(
            std::env::var_os("CI").is_none(),
            "the CI's disks hold the mark: a marked process's file must carry it"
        );
        eprintln!("NOT RUN: this file system cannot hold the mark");
    }
    marks
}

/// Everything under `root`, and `root` itself, that carries the mark.
fn marked(root: &Path) -> Vec<PathBuf> {
    fn walk(path: &Path, out: &mut Vec<PathBuf>) {
        if origin::read(path).unwrap().is_some() {
            out.push(path.to_owned());
        }
        if fs::symlink_metadata(path).unwrap().is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                walk(&entry.unwrap().path(), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

/// Marks everything under `root`, and `root`, as a Zajil marked before this
/// fix left its tools.
fn mark_all(root: &Path) {
    origin::mark(root, "https://example.com").unwrap();
    if fs::symlink_metadata(root).unwrap().is_dir() {
        for entry in fs::read_dir(root).unwrap() {
            mark_all(&entry.unwrap().path());
        }
    }
}

fn https_server() -> TestServer {
    TestServer::https(Tls::trusted(&["localhost", "127.0.0.1"]), "localhost")
}

fn trusting_engine() -> Engine {
    Engine::with_test_root(Options::default(), &test_ca_der()).unwrap()
}

async fn install(toolbox: &Toolbox, tool: Tool) -> PathBuf {
    toolbox
        .install(&trusting_engine(), tool, &Stop::new(), &mut |_| {})
        .await
        .unwrap()
}

/// A zip of `(name, bytes)`, each file executable.
fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut out);
        let options = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        for (name, bytes) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    out.into_inner()
}

#[tokio::test]
async fn a_marked_zajil_s_tools_carry_no_mark() {
    let root = tempfile::tempdir().unwrap();
    if !marked_as_chrome(root.path()) {
        return;
    }
    // A zip (yt-dlp on macOS) and a single file (ffmpeg).
    let entries: [(&str, &[u8]); 3] = [
        ("Python.framework/Python", b"a library"),
        ("Python.framework/Resources/Info.plist", b"its description"),
        ("yt-dlp_test", b"the program"),
    ];
    let zip = zip_of(&entries);
    let ffmpeg = random_bytes(64 * 1024, 7);
    let server = https_server();
    server.put("/yt-dlp.zip", FileSpec::new(zip.clone()));
    server.put("/ffmpeg", FileSpec::new(ffmpeg.clone()));
    let files: Vec<(String, String)> = entries
        .iter()
        .map(|(name, bytes)| ((*name).to_owned(), sha256_hex(bytes)))
        .collect();
    let (zip_url, ffmpeg_url) = (server.url("/yt-dlp.zip"), server.url("/ffmpeg"));
    let tools_dir = root.path().join("tools");
    let toolbox = Toolbox::with_pins(
        &tools_dir,
        Box::new(move |tool, _| match tool {
            Tool::YtDlp => Some(Pinned {
                version: "2099.01.01".to_owned(),
                url: zip_url.clone(),
                sha256: sha256_hex(&zip),
                archive: Some(Archive {
                    folder: "yt-dlp_test".to_owned(),
                    program: "yt-dlp_test".to_owned(),
                    tree_sha256: tree_sha256(&files),
                }),
            }),
            Tool::Ffmpeg => Some(Pinned {
                version: "8.0".to_owned(),
                url: ffmpeg_url.clone(),
                sha256: sha256_hex(&ffmpeg),
                archive: None,
            }),
            _ => None,
        }),
    );

    let yt_dlp = install(&toolbox, Tool::YtDlp).await;
    let ffmpeg = install(&toolbox, Tool::Ffmpeg).await;
    assert_eq!(marked(&tools_dir), Vec::<PathBuf>::new());

    // Tools a marked Zajil unpacked before this fix: the mark is removed
    // before they run.
    mark_all(&tools_dir);
    assert_eq!(toolbox.verified(Tool::YtDlp).await.unwrap().path(), yt_dlp);
    assert_eq!(toolbox.verified(Tool::Ffmpeg).await.unwrap().path(), ffmpeg);
    assert_eq!(marked(&tools_dir), Vec::<PathBuf>::new());
}

/// The real pinned yt-dlp zip, ffmpeg and ffprobe (from the CI, as in
/// video_real.rs), installed through Zajil's own code by a marked process:
/// nothing of them is marked, and yt-dlp runs, its first start too.
#[tokio::test]
async fn a_marked_zajil_s_real_yt_dlp_runs() {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let (Some(yt_dlp), Some(ffmpeg_dir)) = (var("ZAJIL_TEST_YT_DLP"), var("ZAJIL_TEST_FFMPEG_DIR"))
    else {
        assert!(
            std::env::var_os("CI").is_none(),
            "the video tools are missing on the CI"
        );
        eprintln!("NOT RUN: set ZAJIL_TEST_YT_DLP and ZAJIL_TEST_FFMPEG_DIR to run it");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    if !marked_as_chrome(root.path()) {
        return;
    }
    let target = zajil_platform::target::current().unwrap();
    let server = https_server();
    let mut pins = Vec::new();
    for tool in [Tool::YtDlp, Tool::Ffmpeg, Tool::Ffprobe] {
        let pin = zajil_core::tools::pinned(tool, target).unwrap();
        let file = match (tool, &pin.archive) {
            (Tool::YtDlp, Some(archive)) => yt_dlp
                .parent()
                .unwrap()
                .with_file_name(format!("{}.zip", archive.folder)),
            (Tool::YtDlp, None) => panic!("macOS has yt-dlp's zip pinned"),
            (Tool::Ffmpeg, _) => ffmpeg_dir.join("ffmpeg"),
            _ => ffmpeg_dir.join("ffprobe"),
        };
        let path = format!("/{tool}");
        server.put(&path, FileSpec::new(fs::read(&file).unwrap()));
        pins.push((
            tool,
            Pinned {
                url: server.url(&path),
                ..pin
            },
        ));
    }
    let tools_dir = root.path().join("tools");
    let toolbox = || {
        let pins = pins.clone();
        Toolbox::with_pins(
            &tools_dir,
            Box::new(move |tool, _| {
                pins.iter()
                    .find(|(t, _)| *t == tool)
                    .map(|(_, pin)| pin.clone())
            }),
        )
    };
    let tools = toolbox();
    for tool in [Tool::YtDlp, Tool::Ffmpeg, Tool::Ffprobe] {
        install(&tools, tool).await;
    }
    assert_eq!(marked(&tools_dir), Vec::<PathBuf>::new());

    // Its first start, as Zajil makes it right after the install, then a
    // run of its own.
    let runner = VideoRunner::new(
        toolbox(),
        Runs::new(root.path().join("run")),
        trusting_engine(),
    );
    runner.warm_up(&Stop::new()).await.unwrap();
    let checked = tools.verified(Tool::YtDlp).await.unwrap();
    let version = Command::new(checked.path())
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success(), "{version:?}");
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        zajil_core::tools::yt_dlp_version()
    );
    assert_eq!(marked(&tools_dir), Vec::<PathBuf>::new());
}
