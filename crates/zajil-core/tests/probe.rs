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
    assert!(probe.ranges);
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
    // Review of #48: the add dialog says why; only "/no-ranges" lacks ranges.
    for (path, ranges) in [("/no-ranges", false), ("/no-size", true), ("/weak", true)] {
        let probe = engine().probe(&server.url(path)).await.unwrap();
        assert!(!probe.resumable, "{path}");
        assert_eq!(probe.ranges, ranges, "{path}");
    }
}

/// The media type, for the extension's check that a link did not lead to a
/// sign-in page (review of #70): parameters dropped, lower case.
#[tokio::test]
async fn the_content_type_is_read_without_its_parameters() {
    let server = TestServer::http();
    server.put(
        "/page",
        FileSpec::new(b"<html>".to_vec()).content_type("Text/HTML; charset=utf-8"),
    );
    server.put("/file.zip", FileSpec::new(random_bytes(100, 75)));
    let page = engine().probe(&server.url("/page")).await.unwrap();
    assert_eq!(page.content_type.as_deref(), Some("text/html"));
    let file = engine().probe(&server.url("/file.zip")).await.unwrap();
    assert_eq!(
        file.content_type.as_deref(),
        Some("application/octet-stream")
    );
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

/// Review of #48: closing the add dialog gives the probe up, even while it
/// waits to try a failing server again.
#[tokio::test]
async fn a_probe_is_given_up_when_stopped() {
    use zajil_core::Stop;
    let server = TestServer::http();
    server.put(
        "/busy",
        FileSpec::new(random_bytes(1000, 75)).fail_first(vec![
            zajil_testserver::Failure {
                status: 503,
                retry_after: None,
            };
            10
        ]),
    );
    let stop = Stop::new();
    let (engine, url) = (engine(), server.url("/busy"));
    let probing = engine.probe_until(&url, &stop);
    let requested = async {
        // Stop once the server has answered 503 the first time.
        while server.gets("/busy").is_empty() {
            tokio::task::yield_now().await;
        }
        stop.request();
    };
    let (result, ()) = tokio::join!(probing, requested);
    assert!(result.is_err(), "{result:?}");
    assert!(server.gets("/busy").len() < 10);
}

/// The owner's Google Drive link saved a page named «view» and said it was
/// done: a web page is said to be one before it is downloaded (the
/// interface warns), and a name without an extension takes its type's
/// (`view` sent as `text/html` is `view.html`), at the probe and at the
/// download alike. A file is not a page, and a name with an extension
/// keeps it.
#[tokio::test]
async fn a_web_page_is_said_and_named_as_one() {
    let server = TestServer::http();
    server.put(
        "/files/view",
        FileSpec::new("<html><body>A page</body></html>").content_type("text/html; charset=utf-8"),
    );
    server.put(
        "/files/page.xhtml.php",
        FileSpec::new("<html/>").content_type("application/xhtml+xml"),
    );
    server.put(
        "/files/report",
        FileSpec::new(random_bytes(5000, 21)).content_type("application/pdf"),
    );
    server.put(
        "/files/setup",
        FileSpec::new(random_bytes(5000, 22)).content_type("application/x-msdownload"),
    );
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();

    let probe = engine.probe(&server.url("/files/view")).await.unwrap();
    assert!(probe.is_page());
    assert_eq!(probe.content_type.as_deref(), Some("text/html"));
    assert_eq!(probe.name, "view.html");
    let page = engine
        .download(&server.url("/files/view"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(page.file.file_name().unwrap(), "view.html");

    let xhtml = engine
        .probe(&server.url("/files/page.xhtml.php"))
        .await
        .unwrap();
    assert!(xhtml.is_page());
    assert_eq!(xhtml.name, "page.xhtml.php");

    let report = engine.probe(&server.url("/files/report")).await.unwrap();
    assert!(!report.is_page());
    assert_eq!(report.name, "report.pdf");

    // Never an extension that can run code from a server's `Content-Type`.
    let setup = engine.probe(&server.url("/files/setup")).await.unwrap();
    assert_eq!(setup.name, "setup");
}
