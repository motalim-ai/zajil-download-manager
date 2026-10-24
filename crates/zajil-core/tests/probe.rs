//! `Engine::probe` (M4): what the add dialog shows before a download
//! starts. Against the local test server.

use zajil_core::{Engine, Error, Options};
use zajil_testserver::{FileSpec, TestServer, random_bytes};

fn engine() -> Engine {
    Engine::new(Options::default()).unwrap()
}

#[tokio::test]
async fn a_file_that_can_be_split_and_resumed() {
    let server = TestServer::http();
    server.put("/a/report.pdf", FileSpec::new(random_bytes(3_000_000, 70)));
    let probe = engine().probe(&server.url("/a/report.pdf")).await.unwrap();
    assert_eq!(probe.name, "report.pdf");
    assert_eq!(probe.size, Some(3_000_000));
    assert!(probe.resumable);
    assert_eq!(probe.host.as_deref(), Some("127.0.0.1"));
    // Only the headers were read: one request, and no file anywhere.
    assert_eq!(server.gets("/a/report.pdf").len(), 1);
}

#[tokio::test]
async fn the_name_comes_from_content_disposition_and_is_sanitized() {
    let server = TestServer::http();
    server.put(
        "/download",
        FileSpec::new(random_bytes(1000, 71))
            .content_disposition("attachment; filename=\"../../صور الرحلة 2025.zip\""),
    );
    let probe = engine().probe(&server.url("/download")).await.unwrap();
    assert_eq!(probe.name, "صور الرحلة 2025.zip");
}

#[tokio::test]
async fn without_ranges_size_or_a_strong_validator_it_cannot_be_resumed() {
    let server = TestServer::http();
    server.put(
        "/no-ranges",
        FileSpec::new(random_bytes(1000, 72)).without_ranges(),
    );
    server.put(
        "/no-size",
        FileSpec::new(random_bytes(1000, 73)).without_length(),
    );
    server.put(
        "/weak",
        FileSpec::new(random_bytes(1000, 74)).etag(Some("W/\"x\"")),
    );
    for path in ["/no-ranges", "/no-size", "/weak"] {
        let probe = engine().probe(&server.url(path)).await.unwrap();
        assert!(!probe.resumable, "{path}");
    }
}

#[tokio::test]
async fn a_missing_file_is_an_error() {
    let server = TestServer::http();
    let err = engine().probe(&server.url("/nothing")).await.unwrap_err();
    assert!(
        matches!(err, Error::Status(s) if s.as_u16() == 404),
        "{err}"
    );
}
