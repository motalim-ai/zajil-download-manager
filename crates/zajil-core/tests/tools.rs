//! The video tools (ADR-0009): downloaded only when asked, kept only if
//! they are the pinned file, and checked again before every run. Every
//! file is served by the local test server; nothing here reaches the
//! internet.

use std::fs;
use std::path::{Path, PathBuf};

use zajil_core::tools::{Pinned, Tool, ToolError, Toolbox};
use zajil_core::{Engine, Event, Options, Stop};
use zajil_testserver::{FileSpec, TestServer, Tls, random_bytes, sha256_hex, test_ca_der};

fn trusting_engine() -> Engine {
    Engine::with_test_root(Options::default(), &test_ca_der()).unwrap()
}

/// A toolbox in `root` whose yt-dlp is `url` with SHA-256 `sha256`.
fn toolbox(root: &Path, url: String, sha256: String) -> Toolbox {
    Toolbox::with_pins(
        root,
        Box::new(move |tool, _| {
            (tool == Tool::YtDlp).then(|| Pinned {
                version: "2099.01.01".to_owned(),
                url: url.clone(),
                sha256: sha256.clone(),
            })
        }),
    )
}

fn https_server() -> TestServer {
    TestServer::https(Tls::trusted(&["localhost", "127.0.0.1"]), "localhost")
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

async fn install(toolbox: &Toolbox, engine: &Engine) -> Result<PathBuf, ToolError> {
    toolbox
        .install(engine, Tool::YtDlp, &Stop::new(), &mut |_| {})
        .await
}

fn exe(name: &str) -> String {
    format!("{name}{}", if cfg!(windows) { ".exe" } else { "" })
}

#[tokio::test]
async fn installs_the_pinned_file_over_https_and_checks_it_before_use() {
    let server = https_server();
    let data = random_bytes(3 * 1024 * 1024, 61);
    server.put("/yt-dlp", FileSpec::new(data.clone()));
    let root = tempfile::tempdir().unwrap();
    let tools = toolbox(
        &root.path().join("tools"),
        server.url("/yt-dlp"),
        sha256_hex(&data),
    );

    assert!(matches!(
        tools.verified(Tool::YtDlp).await,
        Err(ToolError::Missing(Tool::YtDlp))
    ));
    let path = install(&tools, &trusting_engine()).await.unwrap();

    assert_eq!(fs::read(&path).unwrap(), data);
    assert_eq!(tools.verified(Tool::YtDlp).await.unwrap(), path);
    assert!(tools.is_installed(Tool::YtDlp));
    // Only the tool is left: no download folder, part file or copy.
    assert_eq!(
        tree(&root.path().join("tools")),
        vec![
            "yt-dlp".to_owned(),
            "yt-dlp/2099.01.01".to_owned(),
            format!("yt-dlp/2099.01.01/{}", exe("yt-dlp"))
        ]
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o700);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        assert_eq!(mode(&root.path().join("tools")), 0o700);
    }
}

#[tokio::test]
async fn an_untrusted_certificate_downloads_nothing() {
    // The twin of the test above: the same server, but an engine that
    // trusts only the system's authorities, which do not include the test
    // one (AGENTS.md, section 10.4).
    let server = https_server();
    let data = random_bytes(64 * 1024, 62);
    server.put("/yt-dlp", FileSpec::new(data.clone()));
    let root = tempfile::tempdir().unwrap();
    let tools = toolbox(
        &root.path().join("tools"),
        server.url("/yt-dlp"),
        sha256_hex(&data),
    );

    let err = install(&tools, &Engine::new(Options::default()).unwrap())
        .await
        .expect_err("the certificate must be refused");
    assert!(matches!(err, ToolError::Download(_)), "{err}");
    assert!(
        err.to_string().to_lowercase().contains("certificate"),
        "{err}"
    );
    assert!(server.requests().is_empty());
    assert!(
        tree(&root.path().join("tools")).is_empty(),
        "{:?}",
        tree(&root.path().join("tools"))
    );
}

#[tokio::test]
async fn a_file_that_is_not_the_pinned_one_is_refused_and_removed() {
    let server = https_server();
    let data = random_bytes(512 * 1024, 63);
    server.put("/yt-dlp", FileSpec::new(data.clone()));
    let root = tempfile::tempdir().unwrap();
    let mut other = data.clone();
    other[1000] ^= 1;
    let tools = toolbox(
        &root.path().join("tools"),
        server.url("/yt-dlp"),
        sha256_hex(&other),
    );

    match install(&tools, &trusting_engine()).await {
        Err(ToolError::Mismatch {
            actual, expected, ..
        }) => {
            assert_eq!(actual, sha256_hex(&data));
            assert_eq!(expected, sha256_hex(&other));
        }
        other => panic!("expected a mismatch, got {other:?}"),
    }
    // The folders may stay; no file does.
    let files: Vec<String> = tree(&root.path().join("tools"))
        .into_iter()
        .filter(|p| root.path().join("tools").join(p).is_file())
        .collect();
    assert!(files.is_empty(), "{files:?}");
    assert!(!tools.is_installed(Tool::YtDlp));
}

#[tokio::test]
async fn a_tool_changed_after_it_was_installed_is_not_run() {
    let server = https_server();
    let data = random_bytes(256 * 1024, 64);
    server.put("/yt-dlp", FileSpec::new(data.clone()));
    let root = tempfile::tempdir().unwrap();
    let tools = toolbox(
        &root.path().join("tools"),
        server.url("/yt-dlp"),
        sha256_hex(&data),
    );
    let path = install(&tools, &trusting_engine()).await.unwrap();

    let mut changed = data.clone();
    changed[0] ^= 0xff;
    fs::write(&path, &changed).unwrap();
    assert!(matches!(
        tools.verified(Tool::YtDlp).await,
        Err(ToolError::Mismatch { .. })
    ));
}

#[tokio::test]
async fn a_link_in_place_of_the_tool_is_not_run_and_its_target_is_untouched() {
    let server = https_server();
    let data = random_bytes(128 * 1024, 65);
    server.put("/yt-dlp", FileSpec::new(data.clone()));
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let tools = toolbox(
        &root.path().join("tools"),
        server.url("/yt-dlp"),
        sha256_hex(&data),
    );
    let path = install(&tools, &trusting_engine()).await.unwrap();
    // The pinned bytes outside the tools folder: a link to them would pass
    // a check that only hashed what it opened.
    let victim = outside.path().join("elsewhere");
    fs::write(&victim, &data).unwrap();

    fs::remove_file(&path).unwrap();
    zajil_platform::fs::symlink_file(&victim, &path)
        .expect("this test must be able to create a symbolic link");
    assert!(tools.verified(Tool::YtDlp).await.is_err());
    fs::remove_file(&path).unwrap();

    fs::hard_link(&victim, &path).unwrap();
    assert!(tools.verified(Tool::YtDlp).await.is_err());
    assert_eq!(fs::read(&victim).unwrap(), data);
}

#[tokio::test]
async fn a_link_in_place_of_the_new_file_is_replaced_not_followed() {
    // Something left at `<tool>.new` (the copy's name) is never written
    // through: it is removed, and the copy is a new private file.
    let server = https_server();
    let data = random_bytes(128 * 1024, 66);
    server.put("/yt-dlp", FileSpec::new(data.clone()));
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let tools = toolbox(
        &root.path().join("tools"),
        server.url("/yt-dlp"),
        sha256_hex(&data),
    );
    let dest = tools.path(Tool::YtDlp).unwrap();
    zajil_platform::fs::private_dir(dest.parent().unwrap()).unwrap();
    let victim = outside.path().join("victim");
    fs::write(&victim, b"untouched").unwrap();
    zajil_platform::fs::symlink_file(&victim, &dest.with_extension("new"))
        .expect("this test must be able to create a symbolic link");

    let path = install(&tools, &trusting_engine()).await.unwrap();
    assert_eq!(fs::read(&path).unwrap(), data);
    assert_eq!(fs::read(&victim).unwrap(), b"untouched");
}

#[cfg(unix)]
#[tokio::test]
async fn a_tool_another_user_could_change_is_not_run() {
    // On macOS and Linux a folder the group may write to. The Windows
    // check of the same function (access lists) has its own tests in
    // zajil-platform (#89).
    use std::os::unix::fs::PermissionsExt;
    let server = https_server();
    let data = random_bytes(64 * 1024, 67);
    server.put("/yt-dlp", FileSpec::new(data.clone()));
    let root = tempfile::tempdir().unwrap();
    let tools = toolbox(
        &root.path().join("tools"),
        server.url("/yt-dlp"),
        sha256_hex(&data),
    );
    let path = install(&tools, &trusting_engine()).await.unwrap();

    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o770)).unwrap();
    assert!(matches!(
        tools.verified(Tool::YtDlp).await,
        Err(ToolError::NotPrivate(_))
    ));
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o722)).unwrap();
    assert!(matches!(
        tools.verified(Tool::YtDlp).await,
        Err(ToolError::NotPrivate(_))
    ));
}

#[tokio::test]
async fn a_stopped_download_leaves_nothing_behind() {
    let server = https_server();
    let data = random_bytes(4 * 1024 * 1024, 68);
    server.put("/yt-dlp", FileSpec::new(data.clone()).stall_after(0));
    let root = tempfile::tempdir().unwrap();
    let tools = toolbox(
        &root.path().join("tools"),
        server.url("/yt-dlp"),
        sha256_hex(&data),
    );
    let stop = Stop::new();
    let task_stop = stop.clone();
    let result = tools
        .install(&trusting_engine(), Tool::YtDlp, &stop, &mut |event| {
            if matches!(event, Event::Started { .. }) {
                task_stop.request();
            }
        })
        .await;
    assert!(
        matches!(
            result,
            Err(ToolError::Download(zajil_core::Error::Stopped { .. }))
        ),
        "{result:?}"
    );
    assert!(
        tree(&root.path().join("tools")).is_empty(),
        "{:?}",
        tree(&root.path().join("tools"))
    );
}

#[tokio::test]
async fn a_tool_with_no_released_file_is_never_downloaded() {
    let server = https_server();
    let root = tempfile::tempdir().unwrap();
    let tools = toolbox(
        &root.path().join("tools"),
        server.url("/yt-dlp"),
        "0".repeat(64),
    );
    let err = tools
        .install(&trusting_engine(), Tool::Ffmpeg, &Stop::new(), &mut |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::NotReleased(Tool::Ffmpeg)), "{err}");
    assert!(server.requests().is_empty());
}

#[test]
fn leftover_download_folders_are_removed_and_tools_are_kept() {
    let root = tempfile::tempdir().unwrap();
    let tools_dir = root.path().join("tools");
    fs::create_dir_all(tools_dir.join(".download-0011223344556677")).unwrap();
    fs::write(
        tools_dir.join(".download-0011223344556677/yt-dlp.zajil-part"),
        b"x",
    )
    .unwrap();
    fs::create_dir_all(tools_dir.join("yt-dlp/2099.01.01")).unwrap();
    fs::write(tools_dir.join("yt-dlp/2099.01.01/yt-dlp"), b"tool").unwrap();
    let tools = toolbox(&tools_dir, String::new(), String::new());
    tools.remove_leftovers().unwrap();
    assert_eq!(
        tree(&tools_dir),
        vec!["yt-dlp", "yt-dlp/2099.01.01", "yt-dlp/2099.01.01/yt-dlp"]
    );
}
