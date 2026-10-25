//! Security review #84: every finished download carries the mark of a file
//! from the internet, so the system warns before it runs: `Zone.Identifier`
//! with `ZoneId=3` on Windows, `com.apple.quarantine` on macOS. Its
//! `HostUrl` is the link's origin only, never its path or token.

use zajil_core::{Engine, Options};
use zajil_testserver::{FileSpec, TestServer, random_bytes};

#[tokio::test]
async fn a_finished_download_is_marked_as_from_the_internet() {
    let server = TestServer::http();
    server.put(
        "/secret-path/setup.exe",
        FileSpec::new(random_bytes(100_000, 7)),
    );
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(Options::default()).unwrap();
    let url = format!("{}?token=TOKEN123", server.url("/secret-path/setup.exe"));
    let done = engine
        .download(&url, dir.path(), &mut |_| {})
        .await
        .unwrap();
    let origin = zajil_core::Url::parse(&url)
        .unwrap()
        .origin()
        .ascii_serialization();
    let mark = zajil_platform::origin::read(&done.file).unwrap();
    if cfg!(windows) {
        let mark = mark.expect("no Zone.Identifier on the finished file");
        assert!(mark.contains("ZoneId=3"), "{mark}");
        assert!(mark.contains(&format!("HostUrl={origin}/")), "{mark}");
        assert!(
            !mark.contains("secret-path") && !mark.contains("TOKEN123"),
            "{mark}"
        );
    } else if cfg!(target_os = "macos") {
        let mark = mark.expect("no com.apple.quarantine on the finished file");
        assert!(
            mark.starts_with("0081;") && mark.contains(";Zajil;"),
            "{mark}"
        );
    } else if let Some(mark) = mark {
        // Linux keeps it where the file system allows user attributes.
        assert_eq!(mark, format!("{origin}/"));
    }
}
