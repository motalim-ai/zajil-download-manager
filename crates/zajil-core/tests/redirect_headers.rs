//! What a request carries along a chain of redirects: the browser's cookies
//! and a link's user name and password only to their own origin, at every
//! hop, and the page (`Referer`) as `strict-origin-when-cross-origin` has
//! it for each hop's address, as a browser does.

use zajil_core::{Engine, FromBrowser, Options, Url};
use zajil_testserver::{FileSpec, RequestRecord, TestServer, random_bytes};

fn engine() -> Engine {
    Engine::new(Options {
        connections: 1,
        max_failures: 1,
        ..Options::default()
    })
    .unwrap()
}

/// `server`'s address for `path` on another origin: `localhost` instead of
/// `127.0.0.1`.
fn elsewhere(server: &TestServer, path: &str) -> String {
    server.url(path).replace("127.0.0.1", "localhost")
}

/// A server with a file at `/cdn/f.bin` and `/go`, which redirects to
/// another origin (`/hop` there), which redirects to the file on its own
/// origin.
fn two_hops() -> TestServer {
    let server = TestServer::http();
    server.put("/cdn/f.bin", FileSpec::new(random_bytes(1000, 91)));
    server.redirect("/go", &elsewhere(&server, "/hop"), 302);
    server.redirect("/hop", &elsewhere(&server, "/cdn/f.bin"), 302);
    server
}

/// Every GET the other origin got: `/hop` and the file.
fn elsewhere_gets(server: &TestServer) -> Vec<RequestRecord> {
    let gets = [server.gets("/hop"), server.gets("/cdn/f.bin")].concat();
    assert!(!gets.is_empty());
    gets
}

/// The browser's cookies go to their origin's first request, and to no hop
/// of another origin: not to its first, and not to the next one on that
/// same origin either.
#[tokio::test]
async fn cookies_never_reach_another_origin_on_any_hop() {
    let server = two_hops();
    let from = || FromBrowser::new(&server.url("/go"), Some("sid=SECRET-COOKIE"), None);

    engine()
        .with_browser(from())
        .probe(&server.url("/go"))
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    engine()
        .with_browser(from())
        .download(&server.url("/go"), dir.path(), &mut |_| {})
        .await
        .unwrap();

    for get in server.gets("/go") {
        assert_eq!(get.cookie.as_deref(), Some("sid=SECRET-COOKIE"), "{get:?}");
    }
    for get in elsewhere_gets(&server) {
        assert_eq!(get.cookie, None, "{get:?}");
    }
}

/// A link's user name and password (the `zajil` command takes them) go to
/// their origin only, on every hop.
#[tokio::test]
async fn a_links_password_never_reaches_another_origin_on_any_hop() {
    let server = two_hops();
    let url = server.url("/go").replace("://", "://u:p@");

    engine().probe(&url).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    engine()
        .download(&url, dir.path(), &mut |_| {})
        .await
        .unwrap();

    for get in server.gets("/go") {
        assert!(get.authorization.is_some(), "{get:?}");
    }
    for get in elsewhere_gets(&server) {
        assert_eq!(get.authorization, None, "{get:?}");
    }
}

/// The page's whole address goes to its own origin, and only its origin to
/// another one a redirect leads to.
#[tokio::test]
async fn the_page_goes_whole_only_to_its_own_origin_on_every_hop() {
    let server = two_hops();
    let page = server.url("/page?session=SECRET-PAGE");
    let page_origin = format!(
        "{}/",
        Url::parse(&page).unwrap().origin().ascii_serialization()
    );
    let other_origin = page_origin.replace("127.0.0.1", "localhost");
    let from = || FromBrowser::new(&server.url("/go"), None, Some(&page));

    engine()
        .with_browser(from())
        .probe(&server.url("/go"))
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    engine()
        .with_browser(from())
        .download(&server.url("/go"), dir.path(), &mut |_| {})
        .await
        .unwrap();

    for get in server.gets("/go") {
        assert_eq!(get.referer.as_deref(), Some(page.as_str()), "{get:?}");
    }
    for get in elsewhere_gets(&server) {
        // The page's origin, not the other one's.
        assert_ne!(get.referer.as_deref(), Some(other_origin.as_str()));
        assert_eq!(
            get.referer.as_deref(),
            Some(page_origin.as_str()),
            "{get:?}"
        );
    }
}

/// A redirect on the link's own origin keeps its user name and password,
/// as before.
#[tokio::test]
async fn a_links_password_goes_on_along_its_own_origin() {
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(random_bytes(1000, 92)));
    server.redirect("/go", "/f.bin", 302);
    let url = server.url("/go").replace("://", "://u:p@");

    let dir = tempfile::tempdir().unwrap();
    engine()
        .download(&url, dir.path(), &mut |_| {})
        .await
        .unwrap();

    for get in [server.gets("/go"), server.gets("/f.bin")].concat() {
        assert!(get.authorization.is_some(), "{get:?}");
    }
}
