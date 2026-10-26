//! `Engine::fetch_page` (M7, ADR-0010 section 1): a page into memory,
//! without following redirects, for the site grabber. And the proof that
//! downloads still behave exactly as before next to it.

use std::fs;
use std::time::Duration;

use zajil_core::{Engine, Error, FromBrowser, Options, Stop};
use zajil_testserver::{FileSpec, TestServer, Tls, random_bytes, sha256_hex, test_ca_der};

fn engine() -> Engine {
    Engine::new(Options::default()).unwrap()
}

fn html(body: &str) -> FileSpec {
    FileSpec::new(body.as_bytes().to_vec()).content_type("text/html; charset=utf-8")
}

#[tokio::test]
async fn an_html_page_is_read_into_memory() {
    let server = TestServer::http();
    server.put("/index.html", html("<a href=\"a.zip\">a</a>"));
    let page = engine()
        .fetch_page(&server.url("/index.html"), 1 << 20)
        .await
        .unwrap();
    assert_eq!(page.status, 200);
    assert_eq!(page.content_type.as_deref(), Some("text/html"));
    assert_eq!(page.body.as_deref(), Some(&b"<a href=\"a.zip\">a</a>"[..]));
    assert!(!page.truncated);
    assert_eq!(page.location, None);
}

#[tokio::test]
async fn xhtml_is_a_page_too() {
    let server = TestServer::http();
    server.put(
        "/p",
        FileSpec::new(b"<html/>".to_vec()).content_type("Application/XHTML+XML"),
    );
    let page = engine().fetch_page(&server.url("/p"), 1024).await.unwrap();
    assert_eq!(page.body.as_deref(), Some(&b"<html/>"[..]));
}

/// The redirect comes back to the caller, and its target is not asked for.
#[tokio::test]
async fn a_redirect_is_returned_and_never_followed() {
    let server = TestServer::http();
    server.redirect("/start", "/target.html", 302);
    server.put("/target.html", html("target"));
    let page = engine()
        .fetch_page(&server.url("/start"), 1 << 20)
        .await
        .unwrap();
    assert_eq!(page.status, 302);
    assert_eq!(
        page.location.as_deref(),
        Some(server.path("/target.html").as_str())
    );
    assert_eq!(page.body, None);
    assert!(
        server.gets("/target.html").is_empty(),
        "the target was asked for"
    );
}

/// Owner's condition for this change: downloads behave as before. The same
/// engine follows the redirect for a download (and still refuses https to
/// http, `tls.rs`), while `fetch_page` returns it.
#[tokio::test]
async fn downloads_still_follow_the_redirect_that_fetch_page_returns() {
    let server = TestServer::http();
    let data = random_bytes(300_000, 7);
    server.redirect("/start", "/files/real.iso", 302);
    server.put("/files/real.iso", FileSpec::new(data.clone()));
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();

    let page = engine
        .fetch_page(&server.url("/start"), 1024)
        .await
        .unwrap();
    assert_eq!(page.status, 302);
    let done = engine
        .download(&server.url("/start"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(done.file.file_name().unwrap(), "real.iso");
    assert_eq!(
        sha256_hex(&fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );
    // And a probe too.
    let probe = engine.probe(&server.url("/start")).await.unwrap();
    assert_eq!(probe.name, "real.iso");
}

/// A file that is not HTML: its headers are enough, its body is not read.
#[tokio::test]
async fn a_file_that_is_not_html_is_not_read() {
    let server = TestServer::http();
    server.put(
        "/get/7",
        FileSpec::new(random_bytes(2_000_000, 3))
            .content_type("application/zip")
            .content_disposition("attachment; filename=\"report.zip\""),
    );
    let page = engine()
        .fetch_page(&server.url("/get/7"), 1 << 20)
        .await
        .unwrap();
    assert_eq!(page.status, 200);
    assert_eq!(page.content_type.as_deref(), Some("application/zip"));
    assert_eq!(page.name, "report.zip");
    assert_eq!(page.body, None);
}

#[tokio::test]
async fn a_long_page_is_cut_at_the_limit() {
    let server = TestServer::http();
    server.put("/big.html", html(&"x".repeat(300_000)));
    let page = engine()
        .fetch_page(&server.url("/big.html"), 10_000)
        .await
        .unwrap();
    assert_eq!(page.body.as_ref().map(Vec::len), Some(10_000));
    assert!(page.truncated);
}

/// A page of exactly the size asked for is whole, not cut.
#[tokio::test]
async fn a_page_of_exactly_the_limit_is_not_cut() {
    let server = TestServer::http();
    server.put("/exact.html", html(&"x".repeat(10_000)));
    let page = engine()
        .fetch_page(&server.url("/exact.html"), 10_000)
        .await
        .unwrap();
    assert_eq!(page.body.as_ref().map(Vec::len), Some(10_000));
    assert!(!page.truncated);
}

#[tokio::test]
async fn an_error_status_has_no_body() {
    let server = TestServer::http();
    let page = engine()
        .fetch_page(&server.url("/missing.html"), 1024)
        .await
        .unwrap();
    assert_eq!(page.status, 404);
    assert_eq!(page.body, None);
}

/// The browser's cookies go to their own origin only, as for downloads.
#[tokio::test]
async fn browser_cookies_go_only_to_their_origin() {
    let site = TestServer::http();
    let other = TestServer::http();
    site.put("/p.html", html("site"));
    other.put("/p.html", html("other"));
    let engine = engine().with_browser(FromBrowser::new(&site.url("/"), Some("sid=SECRET"), None));
    engine.fetch_page(&site.url("/p.html"), 1024).await.unwrap();
    engine
        .fetch_page(&other.url("/p.html"), 1024)
        .await
        .unwrap();
    assert_eq!(
        site.gets("/p.html")[0].cookie.as_deref(),
        Some("sid=SECRET")
    );
    assert_eq!(other.gets("/p.html")[0].cookie, None);
}

/// AGENTS.md §10.4: HTTPS succeeds with the test authority, and the same
/// server is refused without it.
#[tokio::test]
async fn https_pages_are_checked_like_downloads() {
    let server = TestServer::https(Tls::trusted(&["localhost"]), "localhost");
    server.put("/p.html", html("secure"));
    let trusting = Engine::with_test_root(Options::default(), &test_ca_der()).unwrap();
    let page = trusting
        .fetch_page(&server.url("/p.html"), 1024)
        .await
        .unwrap();
    assert_eq!(page.body.as_deref(), Some(&b"secure"[..]));
    assert_eq!(
        server.gets("/p.html")[0].tls_version.as_deref(),
        Some("TLSv1_3")
    );

    let err = engine()
        .fetch_page(&server.url("/p.html?sig=SECRET"), 1024)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Http(_)), "{err:?}");
    assert!(!err.to_string().contains("SECRET"), "{err}");
}

/// A stop while the page arrives gives up at once.
#[tokio::test]
async fn a_stop_gives_up_the_page() {
    let server = TestServer::http();
    server.put("/slow.html", html(&"x".repeat(100_000)).stall_after(1_000));
    let stop = Stop::new();
    let later = stop.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        later.request();
    });
    let err = engine()
        .fetch_page_until(&server.url("/slow.html"), 1 << 20, &stop)
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Stopped { state_file: None }),
        "{err:?}"
    );
}

/// `robots.txt` is plain text: `fetch_robots_until` reads it, `fetch_page`
/// does not, and neither follows a redirect.
#[tokio::test]
async fn fetch_robots_reads_any_type() {
    let server = TestServer::http();
    server.put(
        "/robots.txt",
        FileSpec::new(b"User-agent: *\nDisallow: /x\n".to_vec()).content_type("text/plain"),
    );
    server.put("/untyped", FileSpec::new(b"plain".to_vec()));
    let engine = engine();
    let stop = Stop::new();
    let text = engine
        .fetch_robots_until(&server.url("/robots.txt"), 1024, &stop)
        .await
        .unwrap();
    assert_eq!(
        text.body.as_deref(),
        Some(&b"User-agent: *\nDisallow: /x\n"[..])
    );
    let page = engine
        .fetch_page(&server.url("/robots.txt"), 1024)
        .await
        .unwrap();
    assert_eq!(page.body, None);
    let untyped = engine
        .fetch_robots_until(&server.url("/untyped"), 3, &stop)
        .await
        .unwrap();
    assert_eq!(untyped.body.as_deref(), Some(&b"pla"[..]));
    assert!(untyped.truncated);
    server.redirect("/moved.txt", "/robots.txt", 301);
    let moved = engine
        .fetch_robots_until(&server.url("/moved.txt"), 1024, &stop)
        .await
        .unwrap();
    assert_eq!(moved.status, 301);
    assert_eq!(moved.body, None);
}
