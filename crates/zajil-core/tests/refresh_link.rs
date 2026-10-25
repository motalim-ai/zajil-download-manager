//! "Refresh the download link" (M5): a download whose link expired goes on
//! from a new link to the same file, and a link to another file is refused.
//! And the browser's headers (`FromBrowser`): the cookies go only to their
//! origin, and never into the state file.

use std::fs;
use std::path::{Path, PathBuf};

use zajil_core::{Engine, Error, FromBrowser, Options};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

const SIZE: usize = 4 * 1024 * 1024;
const CUT: u64 = 1024 * 1024;

fn engine() -> Engine {
    Engine::new(Options {
        connections: 1,
        max_failures: 1,
        ..Options::default()
    })
    .unwrap()
}

/// A download of `path` that the server cuts at `CUT`: its state file.
async fn cut_download(server: &TestServer, path: &str, dir: &Path) -> PathBuf {
    match engine().download(&server.url(path), dir, &mut |_| {}).await {
        Err(Error::Interrupted { state_file, .. }) => state_file,
        other => panic!("expected an interruption, got {other:?}"),
    }
}

/// The ROADMAP criterion: a download starts on a link that expires in the
/// middle; given a new link to the same file, it goes on from where it was
/// (only the missing bytes are asked for) and the file is whole.
#[tokio::test]
async fn a_download_goes_on_from_a_new_link_to_the_same_file() {
    let server = TestServer::http();
    let data = random_bytes(SIZE, 81);
    let spec = || FileSpec::new(data.clone()).etag(Some("\"v1\""));
    server.put("/token1/f.bin", spec().cut_after(CUT, 1));
    let dir = tempfile::tempdir().unwrap();
    let state_file = cut_download(&server, "/token1/f.bin", dir.path()).await;

    // The signed link expired: resuming from it is refused by the server.
    server.put("/token1/f.bin", FileSpec::new(Vec::new()).forbid_after(0));
    assert!(engine().resume(&state_file, &mut |_| {}).await.is_err());

    server.put("/token2/f.bin", spec());
    engine()
        .refresh_link(&state_file, &server.url("/token2/f.bin"))
        .await
        .unwrap();
    let finished = engine().resume(&state_file, &mut |_| {}).await.unwrap();
    assert_eq!(
        sha256_hex(&fs::read(&finished.file).unwrap()),
        sha256_hex(&data)
    );
    // The refresh read the headers only; the resume asked for the rest.
    let gets = server.gets("/token2/f.bin");
    assert_eq!(gets.len(), 2, "{gets:?}");
    assert_eq!(gets[0].range, None);
    let range = gets[1].range.as_deref().unwrap();
    assert!(
        range.starts_with("bytes=") && !range.starts_with("bytes=0-"),
        "{range}"
    );
    assert_eq!(gets[1].status, 206);
}

/// A link to another file (another validator, or another size) is refused,
/// and the state file is left exactly as it was.
#[tokio::test]
async fn a_link_to_another_file_is_refused() {
    let server = TestServer::http();
    let data = random_bytes(SIZE, 82);
    server.put(
        "/a/f.bin",
        FileSpec::new(data.clone())
            .etag(Some("\"v1\""))
            .cut_after(CUT, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = cut_download(&server, "/a/f.bin", dir.path()).await;
    let before = fs::read(&state_file).unwrap();

    let mut other = data.clone();
    other[10] ^= 0xff;
    server.put("/other-version", FileSpec::new(other).etag(Some("\"v2\"")));
    server.put(
        "/other-size",
        FileSpec::new(random_bytes(SIZE - 1, 83)).etag(Some("\"v1\"")),
    );
    server.put("/gone", FileSpec::new(Vec::new()).forbid_after(0));
    for path in ["/other-version", "/other-size"] {
        let err = engine()
            .refresh_link(&state_file, &server.url(path))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::NotTheSameFile(_)), "{path}: {err}");
    }
    let err = engine()
        .refresh_link(&state_file, &server.url("/gone"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Status(s) if s.as_u16() == 403),
        "{err}"
    );
    let err = engine()
        .refresh_link(&state_file, "https://user:pw@127.0.0.1:9/f.bin")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::InvalidUrl { reason, .. } if reason.contains("password")),
        "{err}"
    );
    let err = engine()
        .refresh_link(&state_file, "ftp://127.0.0.1:9/f.bin")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::UnsupportedScheme(_)), "{err}");
    assert_eq!(
        fs::read(&state_file).unwrap(),
        before,
        "the state file changed"
    );
}

/// A new link that redirects: the state keeps the new link and its target,
/// the target without a user name or password (review of #74), and no
/// credentials flag; the download then completes.
#[tokio::test]
async fn a_new_link_that_redirects_is_kept_without_credentials() {
    let server = TestServer::http();
    let data = random_bytes(SIZE, 88);
    let spec = || FileSpec::new(data.clone()).etag(Some("\"v1\""));
    server.put("/a/f.bin", spec().cut_after(CUT, 1));
    let dir = tempfile::tempdir().unwrap();
    let state_file = cut_download(&server, "/a/f.bin", dir.path()).await;
    let target = server
        .url("/cdn/f.bin")
        .replace("http://", "http://cdnuser:CDNPASS@");
    server.redirect("/new", &target, 302);
    server.put("/cdn/f.bin", spec());
    engine()
        .refresh_link(&state_file, &server.url("/new"))
        .await
        .unwrap();
    let text = fs::read_to_string(&state_file).unwrap();
    assert!(
        !text.contains("CDNPASS") && !text.contains("cdnuser"),
        "{text}"
    );
    let state = zajil_core::state::DownloadState::load(&state_file).unwrap();
    assert_eq!(state.url, server.url("/new"));
    assert_eq!(state.final_url, server.url("/cdn/f.bin"));
    assert!(!state.had_credentials);
    let finished = engine().resume(&state_file, &mut |_| {}).await.unwrap();
    assert_eq!(fs::read(finished.file).unwrap(), data);
}

/// With the part file missing, a refresh still holds its lock: a resume
/// that starts meanwhile is refused, and a refresh that fails leaves no
/// part file behind (review of #74).
#[tokio::test]
async fn a_refresh_without_a_part_file_still_holds_the_lock() {
    let server = TestServer::http();
    let data = random_bytes(SIZE, 89);
    server.put(
        "/a/f.bin",
        FileSpec::new(data.clone())
            .etag(Some("\"v1\""))
            .cut_after(CUT, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = cut_download(&server, "/a/f.bin", dir.path()).await;
    let part = state_file.with_file_name("f.bin.zajil-part");
    fs::remove_file(&part).unwrap();

    // A new link that never answers: the refresh waits on it with the
    // lock held (a listener that accepts and keeps quiet).
    let quiet = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let quiet_url = format!("http://{}/f.bin", quiet.local_addr().unwrap());
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in quiet.incoming().flatten() {
            held.push(stream);
        }
    });
    let refresh = {
        let state_file = state_file.clone();
        tokio::spawn(async move { engine().refresh_link(&state_file, &quiet_url).await })
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !part.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "no part file to hold the lock"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let err = engine().resume(&state_file, &mut |_| {}).await.unwrap_err();
    assert!(matches!(err, Error::Locked(_)), "{err}");
    refresh.abort();
    let _ = refresh.await;

    // A refresh that fails (another file) removes the part file it made.
    let _ = fs::remove_file(&part);
    server.put(
        "/other.bin",
        FileSpec::new(random_bytes(SIZE, 90)).etag(Some("\"v2\"")),
    );
    assert!(
        engine()
            .refresh_link(&state_file, &server.url("/other.bin"))
            .await
            .is_err()
    );
    assert!(!part.exists(), "the part file made for the lock was left");
}

/// Without a strong validator Zajil cannot tell another link is the same
/// file: refused, even when the size matches.
#[tokio::test]
async fn without_a_strong_validator_a_new_link_is_refused() {
    let server = TestServer::http();
    let data = random_bytes(SIZE, 84);
    let weak = || {
        FileSpec::new(data.clone())
            .etag(Some("W/\"weak\""))
            .last_modified(None)
    };
    server.put("/a/f.bin", weak().cut_after(CUT, 1));
    let dir = tempfile::tempdir().unwrap();
    let state_file = cut_download(&server, "/a/f.bin", dir.path()).await;
    server.put("/b/f.bin", weak());
    let err = engine()
        .refresh_link(&state_file, &server.url("/b/f.bin"))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::NotTheSameFile(why) if why.contains("validator")),
        "{err}"
    );
}

/// A download that is running (its part file locked) is not changed.
#[tokio::test]
async fn a_running_download_is_not_changed() {
    let server = TestServer::http();
    let data = random_bytes(SIZE, 85);
    server.put(
        "/a/f.bin",
        FileSpec::new(data.clone())
            .etag(Some("\"v1\""))
            .cut_after(CUT, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = cut_download(&server, "/a/f.bin", dir.path()).await;
    let part = state_file.with_file_name("f.bin.zajil-part");
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&part)
        .unwrap();
    held.lock().unwrap();
    server.put("/b/f.bin", FileSpec::new(data).etag(Some("\"v1\"")));
    let err = engine()
        .refresh_link(&state_file, &server.url("/b/f.bin"))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Locked(_)), "{err}");
}

/// The browser's cookies go with every request to their origin, the page
/// with every request; a redirect to another origin gets no cookies; and
/// the state file never holds them.
#[tokio::test]
async fn the_browser_s_cookies_go_only_to_their_origin_and_never_to_disk() {
    let server = TestServer::http();
    let data = random_bytes(SIZE, 86);
    server.put(
        "/f.bin",
        FileSpec::new(data.clone())
            .etag(Some("\"v1\""))
            .cut_after(CUT, 1),
    );
    // The same server under another host name: another origin.
    let elsewhere = server.url("/cdn/f.bin").replace("127.0.0.1", "localhost");
    server.redirect("/go", &elsewhere, 302);
    server.put(
        "/cdn/f.bin",
        FileSpec::new(data.clone()).etag(Some("\"v1\"")),
    );
    let url = server.url("/f.bin");
    // A page of the same origin: sent whole.
    let page = server.url("/page?q=1");
    let from = FromBrowser::new(&url, Some("sid=SECRET-COOKIE"), Some(&page));
    let browser = engine().with_browser(from.clone());
    let dir = tempfile::tempdir().unwrap();
    let state_file = match browser.download(&url, dir.path(), &mut |_| {}).await {
        Err(Error::Interrupted { state_file, .. }) => state_file,
        other => panic!("expected an interruption, got {other:?}"),
    };
    assert!(!String::from_utf8_lossy(&fs::read(&state_file).unwrap()).contains("SECRET-COOKIE"));
    let finished = browser.resume(&state_file, &mut |_| {}).await.unwrap();
    assert_eq!(fs::read(finished.file).unwrap(), data);
    for get in server.gets("/f.bin") {
        assert_eq!(get.cookie.as_deref(), Some("sid=SECRET-COOKIE"), "{get:?}");
        assert_eq!(get.referer.as_deref(), Some(page.as_str()));
    }

    // Through a redirect to another origin: the page goes, the cookies not.
    let dir = tempfile::tempdir().unwrap();
    let from = FromBrowser::new(
        &server.url("/go"),
        Some("sid=SECRET-COOKIE"),
        Some("https://example.com/page"),
    );
    engine()
        .with_browser(from)
        .download(&server.url("/go"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    // On several connections to the same origin, every ranged request
    // carries them too (review of #74).
    let dir = tempfile::tempdir().unwrap();
    server.put(
        "/split.bin",
        FileSpec::new(data.clone()).etag(Some("\"v1\"")),
    );
    let split_url = server.url("/split.bin");
    let from = FromBrowser::new(
        &split_url,
        Some("sid=SECRET-COOKIE"),
        Some("https://example.com/page"),
    );
    Engine::new(Options {
        connections: 4,
        min_segment_size: 256 * 1024,
        ..Options::default()
    })
    .unwrap()
    .with_browser(from)
    .download(&split_url, dir.path(), &mut |_| {})
    .await
    .unwrap();
    let gets = server.gets("/split.bin");
    assert!(
        gets.iter().filter(|g| g.range.is_some()).count() >= 2,
        "{gets:?}"
    );
    for get in gets {
        assert_eq!(get.cookie.as_deref(), Some("sid=SECRET-COOKIE"), "{get:?}");
        // http here, so a page on https is not sent (a downgrade).
        assert_eq!(get.referer, None, "{get:?}");
    }

    // With several connections the segments go straight to the final
    // address, outside the redirect: still no cookies there.
    let dir = tempfile::tempdir().unwrap();
    let from = FromBrowser::new(&server.url("/go"), Some("sid=SECRET-COOKIE"), None);
    let split = Engine::new(Options {
        connections: 4,
        min_segment_size: 256 * 1024,
        ..Options::default()
    })
    .unwrap();
    split
        .with_browser(from)
        .download(&server.url("/go"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    let cdn = server.gets("/cdn/f.bin");
    assert!(
        cdn.iter().filter(|g| g.range.is_some()).count() >= 2,
        "{cdn:?}"
    );
    for get in cdn {
        assert_eq!(get.cookie, None, "{get:?}");
    }
    // An engine without them sends neither.
    let dir = tempfile::tempdir().unwrap();
    server.put("/plain.bin", FileSpec::new(random_bytes(1000, 87)));
    engine()
        .download(&server.url("/plain.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    let plain = &server.gets("/plain.bin")[0];
    assert_eq!(
        (plain.cookie.as_deref(), plain.referer.as_deref()),
        (None, None)
    );
}
