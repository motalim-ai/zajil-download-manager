//! Google Drive links and web pages: a file's page link is downloaded from
//! Drive's download address, the page Drive answers a large file with is
//! followed to the file, a page that is not the file is never saved, and
//! a name without an extension takes its type's. The local test server
//! plays Drive (`Engine::with_test_drive`): no request leaves the computer.

use std::fs;
use std::path::Path;

use zajil_core::{Engine, Error, FromBrowser, Options, Stop};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

const ID: &str = "1AbC-dEf_GhIjKlMnOpQrStUvWxYz0123";

/// An engine for which `server` is Google Drive.
fn engine(server: &TestServer) -> Engine {
    Engine::new(Options::default())
        .unwrap()
        .with_test_drive(&server.url("/"), &server.url("/download"))
}

fn direct_path(id: &str) -> String {
    format!("/download?id={id}&export=download")
}

/// The page Drive answers a file too large to scan with, as it is made:
/// a form to the download address with the file's id, `confirm` and a
/// `uuid`.
fn confirm_page(server: &TestServer, id: &str, action: &str) -> FileSpec {
    let _ = server;
    confirm_page_with(id, action, UUID)
}

const UUID: &str = "4f1c2d3e-aaaa-bbbb-cccc-0123456789ab";

/// [`confirm_page`] with another `uuid` (a new page Drive makes later).
fn confirm_page_with(id: &str, action: &str, uuid: &str) -> FileSpec {
    FileSpec::new(format!(
        r#"<!DOCTYPE html><html><head><title>Google Drive - Virus scan warning</title></head><body>
        <p>Google Drive can't scan this file for viruses.</p>
        <form id="download-form" action="{action}" method="get">
        <input type="submit" id="uc-download-link" value="Download anyway"/>
        <input type="hidden" name="id" value="{id}"><input type="hidden" name="export" value="download">
        <input type="hidden" name="confirm" value="t"><input type="hidden" name="uuid" value="{uuid}">
        </form></body></html>"#
    ))
    .content_type("text/html; charset=utf-8")
}

const CONFIRMED_QUERY: &str = "&confirm=t&uuid=4f1c2d3e-aaaa-bbbb-cccc-0123456789ab";

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// A small file: its page link (`/file/d/<id>/view`) is downloaded from the
/// download address at once; the page itself is never asked for.
#[tokio::test]
async fn a_drive_file_s_page_link_downloads_the_file() {
    let server = TestServer::http();
    let data = random_bytes(200_000, 801);
    server.put(
        &direct_path(ID),
        FileSpec::new(data.clone())
            .content_disposition("attachment; filename=\"تقرير.pdf\"")
            .content_type("application/pdf"),
    );
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&server);
    for link in [
        format!("/file/d/{ID}/view?usp=sharing"),
        format!("/open?id={ID}"),
        format!("/uc?id={ID}&export=download"),
    ] {
        let probe = engine.probe(&server.url(&link)).await.unwrap();
        assert_eq!(probe.name, "تقرير.pdf", "{link}");
        assert!(!probe.is_page(), "{link}");
        let finished = engine
            .download(&server.url(&link), dir.path(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(
            sha256_hex(&fs::read(&finished.file).unwrap()),
            sha256_hex(&data)
        );
        fs::remove_file(&finished.file).unwrap();
    }
    for request in server.requests() {
        assert_eq!(request.path, direct_path(ID), "{request:?}");
        assert_eq!(request.cookie, None, "a cookie was sent: {request:?}");
    }
}

/// A large file: Drive answers its download address with a page; its form
/// gives the real link, which is downloaded, split and resumed like any
/// file. Nothing of the page is saved, and no cookie is ever sent.
#[tokio::test]
async fn the_page_drive_answers_a_large_file_with_leads_to_the_file() {
    let server = TestServer::http();
    let data = random_bytes(4 * 1024 * 1024, 802);
    server.put(
        &direct_path(ID),
        confirm_page(&server, ID, &server.url("/download")),
    );
    let confirmed = format!("{}{CONFIRMED_QUERY}", direct_path(ID));
    server.put(
        &confirmed,
        FileSpec::new(data.clone())
            .content_disposition("attachment; filename=\"big.zip\"")
            .content_type("application/zip")
            .throttle(4 * 1024 * 1024),
    );
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&server);
    let link = server.url(&format!("/file/d/{ID}/view"));

    let probe = engine.probe(&link).await.unwrap();
    assert_eq!((probe.name.as_str(), probe.is_page()), ("big.zip", false));

    // Stopped in the middle, then resumed from its state file.
    let stop = Stop::new();
    let stopping = stop.clone();
    let err = engine
        .download_until(&link, dir.path(), &stop, &mut |event| {
            if let zajil_core::Event::Progress { done, .. } = event
                && done > 1024 * 1024
            {
                stopping.request();
            }
        })
        .await
        .unwrap_err();
    let Error::Stopped {
        state_file: Some(state_file),
    } = err
    else {
        panic!("{err}");
    };
    let before = server.gets(&confirmed).len();
    let mut restarted = false;
    let finished = engine
        .resume_until(&state_file, &Stop::new(), &mut |event| {
            restarted |= matches!(event, zajil_core::Event::Restarted { .. });
        })
        .await
        .unwrap();
    // Resumed where it was, not started over.
    assert!(!restarted, "it started over");
    let resumed = server.gets(&confirmed);
    assert!(
        resumed[before..].iter().any(|g| g
            .range
            .as_deref()
            .is_some_and(|r| !r.starts_with("bytes=0-"))),
        "nothing was asked from where it stopped: {resumed:?}"
    );
    assert_eq!(names_in(dir.path()), ["big.zip"]);
    assert_eq!(
        sha256_hex(&fs::read(&finished.file).unwrap()),
        sha256_hex(&data)
    );
    assert!(server.requests().iter().all(|r| r.cookie.is_none()));
}

/// The extension's cookies go to their own origin only: Drive's file
/// pages and its download address are two origins, so the cookies given
/// with a file's page link are not sent to the download address. (Two
/// test servers, as Drive's two hosts.)
#[tokio::test]
async fn the_extension_s_cookies_never_leave_their_origin() {
    let links = TestServer::http();
    let download = TestServer::http();
    download.put(
        &direct_path(ID),
        FileSpec::new(random_bytes(10_000, 803))
            .content_disposition("attachment; filename=\"a.bin\""),
    );
    let dir = tempfile::tempdir().unwrap();
    let link = links.url(&format!("/file/d/{ID}/view"));
    let engine = Engine::new(Options::default())
        .unwrap()
        .with_test_drive(&links.url("/"), &download.url("/download"))
        .with_browser(FromBrowser::new(&link, Some("SID=from-the-browser"), None));
    engine
        .download(&link, dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert!(links.requests().is_empty(), "{:?}", links.requests());
    let requests = download.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(
        requests[0].cookie, None,
        "the browser's cookies went elsewhere"
    );

    // Given for the download address itself (the browser's own download,
    // sent by the extension), they go with it.
    let direct = download.url(&direct_path(ID));
    let engine = engine.with_browser(FromBrowser::new(
        &direct,
        Some("SID=from-the-browser"),
        None,
    ));
    engine
        .download(&direct, dir.path(), &mut |_| {})
        .await
        .unwrap();
    let last = download.requests().pop().unwrap();
    assert_eq!(last.cookie.as_deref(), Some("SID=from-the-browser"));
}

/// A page with no link to the file (a private file's sign-in page, a
/// file over its download limit), or a link Drive did not make, or a page
/// again after the link: an error, and nothing is made. The other link is
/// never asked for.
#[tokio::test]
async fn a_drive_page_that_is_not_the_file_is_never_saved() {
    let server = TestServer::http();
    let page = |html: &str| FileSpec::new(html.to_owned()).content_type("text/html");
    server.put(
        &direct_path("1PrivateFile0123456789"),
        page("<html><body>Sign in to continue to Google Drive</body></html>"),
    );
    server.put(
        &direct_path("1ElsewhereFile0123456789"),
        confirm_page(
            &server,
            "1ElsewhereFile0123456789",
            &server.url("/elsewhere"),
        ),
    );
    server.put(
        &format!("/elsewhere?id=1ElsewhereFile0123456789&export=download{CONFIRMED_QUERY}"),
        FileSpec::new(random_bytes(1000, 804)),
    );
    server.put(
        &direct_path("1PageAgain0123456789"),
        confirm_page(&server, "1PageAgain0123456789", &server.url("/download")),
    );
    server.put(
        &format!("{}{CONFIRMED_QUERY}", direct_path("1PageAgain0123456789")),
        page("<html><body>Quota exceeded</body></html>"),
    );
    // A private file: Drive sends it to its sign-in page, on another
    // address (review: such a page was saved as the file). The page even
    // has a form that looks like Drive's.
    server.redirect(
        &direct_path("1SignInFile0123456789"),
        "/ServiceLogin?continue=x",
        302,
    );
    server.put(
        "/ServiceLogin",
        page(&format!(
            "<html><body><form action='{}'><input name='id' value='1SignInFile0123456789'><input name='confirm' value='t'></form></body></html>",
            server.url("/download")
        )),
    );
    // A page too large to be Drive's: not read.
    server.put(
        &direct_path("1HugePage0123456789"),
        page(&format!(
            "{}<form action='{}'><input name='id' value='1HugePage0123456789'><input name='confirm' value='t'></form>",
            " ".repeat(600 * 1024),
            server.url("/download")
        )),
    );
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&server);
    for id in [
        "1PrivateFile0123456789",
        "1ElsewhereFile0123456789",
        "1PageAgain0123456789",
        "1SignInFile0123456789",
        "1HugePage0123456789",
    ] {
        let link = server.url(&format!("/file/d/{id}/view"));
        let probed = engine.probe(&link).await;
        assert!(matches!(probed, Err(Error::DrivePage)), "{id}: {probed:?}");
        let downloaded = engine.download(&link, dir.path(), &mut |_| {}).await;
        assert!(
            matches!(downloaded, Err(Error::DrivePage)),
            "{id}: {downloaded:?}"
        );
    }
    assert!(
        names_in(dir.path()).is_empty(),
        "{:?}",
        names_in(dir.path())
    );
    assert!(
        server
            .requests()
            .iter()
            .all(|r| !r.path.starts_with("/elsewhere") && !r.path.starts_with("/signin")),
        "a link Drive did not make was followed"
    );
}

/// A web page is said to be one before it is downloaded (the interface
/// warns), and a name without an extension takes its type's: `view` sent
/// as `text/html` is `view.html`. A Drive file that is a web page comes
/// with its name, and is a file like any other.
#[tokio::test]
async fn a_web_page_is_said_and_named_as_one() {
    let server = TestServer::http();
    server.put(
        "/files/view",
        FileSpec::new("<html><body>A page</body></html>").content_type("text/html; charset=utf-8"),
    );
    server.put(
        "/files/report",
        FileSpec::new(random_bytes(5000, 805)).content_type("application/pdf"),
    );
    server.put(
        &direct_path("1HtmlFileOnDrive0123"),
        FileSpec::new("<html><body>Notes</body></html>")
            .content_type("text/html")
            .content_disposition("attachment; filename=\"notes.html\""),
    );
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&server);

    let probe = engine.probe(&server.url("/files/view")).await.unwrap();
    assert!(probe.is_page());
    assert_eq!(probe.content_type.as_deref(), Some("text/html"));
    assert_eq!(probe.name, "view.html");
    let page = engine
        .download(&server.url("/files/view"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(page.file.file_name().unwrap(), "view.html");

    let probe = engine.probe(&server.url("/files/report")).await.unwrap();
    assert!(!probe.is_page());
    assert_eq!(probe.name, "report.pdf");

    let link = server.url("/file/d/1HtmlFileOnDrive0123/view");
    let notes = engine
        .download(&link, dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(notes.file.file_name().unwrap(), "notes.html");
    assert_eq!(
        fs::read(&notes.file).unwrap(),
        b"<html><body>Notes</body></html>"
    );
}

/// No request follows a sign-in page's form, nor a too large page's.
#[tokio::test]
async fn nothing_follows_a_page_that_is_not_drive_s() {
    let server = TestServer::http();
    server.redirect(
        &direct_path("1SignInFile0123456789"),
        "/ServiceLogin?continue=x",
        302,
    );
    server.put(
        "/ServiceLogin",
        FileSpec::new(format!(
            "<form action='{}'><input name='id' value='1SignInFile0123456789'><input name='confirm' value='t'></form>",
            server.url("/download")
        ))
        .content_type("text/html"),
    );
    let dir = tempfile::tempdir().unwrap();
    let link = server.url("/file/d/1SignInFile0123456789/view");
    let result = engine(&server)
        .download(&link, dir.path(), &mut |_| {})
        .await;
    assert!(matches!(result, Err(Error::DrivePage)), "{result:?}");
    let paths: Vec<String> = server.requests().into_iter().map(|r| r.path).collect();
    assert_eq!(
        paths,
        [
            direct_path("1SignInFile0123456789"),
            "/ServiceLogin".to_owned()
        ]
    );
    assert!(names_in(dir.path()).is_empty());
}

/// The older page Drive made: a link on its file pages' address
/// (`/uc?export=download&amp;confirm=…&amp;id=…`), followed to the
/// download address.
#[tokio::test]
async fn drive_s_older_page_with_a_link_leads_to_the_file() {
    let server = TestServer::http();
    let data = random_bytes(30_000, 806);
    server.put(
        &direct_path(ID),
        FileSpec::new(format!(
            r#"<html><body><a id="uc-download-link" href="{}?export=download&amp;confirm=AbCd&amp;id={ID}">Download anyway</a></body></html>"#,
            server.path("/uc")
        ))
        .content_type("text/html"),
    );
    server.put(
        &format!("/download?export=download&confirm=AbCd&id={ID}"),
        FileSpec::new(data.clone()).content_disposition("attachment; filename=\"old.bin\""),
    );
    let dir = tempfile::tempdir().unwrap();
    let finished = engine(&server)
        .download(
            &server.url(&format!("/open?id={ID}")),
            dir.path(),
            &mut |_| {},
        )
        .await
        .unwrap();
    assert_eq!(finished.file.file_name().unwrap(), "old.bin");
    assert_eq!(
        sha256_hex(&fs::read(&finished.file).unwrap()),
        sha256_hex(&data)
    );
    assert!(
        server.gets("/uc").is_empty(),
        "the page's own address was asked"
    );
}

/// Review of the Drive links: a connection of a split download that Drive
/// answers with a page (a new confirm page, a quota page) must not write
/// it into the file. The download starts over through the confirm step,
/// and the file is whole.
#[tokio::test]
async fn a_page_answered_to_a_connection_is_never_written() {
    let server = TestServer::http();
    let data = random_bytes(6 * 1024 * 1024, 807);
    let file = |data: &[u8]| {
        FileSpec::new(data.to_vec())
            .content_disposition("attachment; filename=\"split.bin\"")
            .throttle(8 * 1024 * 1024)
    };
    server.put(
        &direct_path(ID),
        confirm_page(&server, ID, &server.url("/download")),
    );
    // After its first answer, the confirmed link answers with a new
    // confirm page, which leads to the file again.
    let other_uuid = "9e8d7c6b-1111-2222-3333-444455556666";
    server.put(
        &format!("{}{CONFIRMED_QUERY}", direct_path(ID)),
        file(&data).replaced_after(
            1,
            confirm_page_with(ID, &server.url("/download"), other_uuid),
        ),
    );
    let again = format!("{}&confirm=t&uuid={other_uuid}", direct_path(ID));
    server.put(&again, file(&data));
    let dir = tempfile::tempdir().unwrap();
    let finished = engine(&server)
        .download(
            &server.url(&format!("/file/d/{ID}/view")),
            dir.path(),
            &mut |_| {},
        )
        .await
        .unwrap();
    assert_eq!(names_in(dir.path()), ["split.bin"]);
    assert_eq!(
        sha256_hex(&fs::read(&finished.file).unwrap()),
        sha256_hex(&data)
    );
    assert!(
        !server.gets(&again).is_empty(),
        "it did not start over through the confirm step"
    );
}

/// A confirmed link that no longer works (Drive's links expire): the
/// resume goes back to the file's page link, through the confirm step,
/// to the file.
#[tokio::test]
async fn an_expired_confirmed_link_is_confirmed_again() {
    let server = TestServer::http();
    let data = random_bytes(3 * 1024 * 1024, 808);
    server.put(
        &direct_path(ID),
        confirm_page(&server, ID, &server.url("/download")),
    );
    let confirmed = format!("{}{CONFIRMED_QUERY}", direct_path(ID));
    server.put(
        &confirmed,
        FileSpec::new(data.clone())
            .content_disposition("attachment; filename=\"expires.bin\"")
            .throttle(2 * 1024 * 1024)
            .forbid_after(1),
    );
    let dir = tempfile::tempdir().unwrap();
    // One connection: its first answer is the only one before it expires.
    let engine = Engine::new(Options {
        connections: 1,
        ..Options::default()
    })
    .unwrap()
    .with_test_drive(&server.url("/"), &server.url("/download"));
    let link = server.url(&format!("/file/d/{ID}/view"));
    let stop = Stop::new();
    let stopping = stop.clone();
    let err = engine
        .download_until(&link, dir.path(), &stop, &mut |event| {
            if let zajil_core::Event::Progress { done, .. } = event
                && done > 512 * 1024
            {
                stopping.request();
            }
        })
        .await
        .unwrap_err();
    let Error::Stopped {
        state_file: Some(state_file),
    } = err
    else {
        panic!("{err}");
    };
    // Drive makes a new page, with a new link, for the file.
    let other_uuid = "0a1b2c3d-5555-6666-7777-888899990000";
    server.put(
        &direct_path(ID),
        confirm_page_with(ID, &server.url("/download"), other_uuid),
    );
    let again = format!("{}&confirm=t&uuid={other_uuid}", direct_path(ID));
    server.put(
        &again,
        FileSpec::new(data.clone()).content_disposition("attachment; filename=\"expires.bin\""),
    );
    let finished = engine
        .resume_until(&state_file, &Stop::new(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(
        sha256_hex(&fs::read(&finished.file).unwrap()),
        sha256_hex(&data)
    );
    assert!(!server.gets(&again).is_empty());
}

/// What an interface asks the browser's cookies for: the real download
/// address of a Drive file's page, and nothing for any other link.
#[test]
fn a_drive_file_link_names_its_download_address() {
    let id = "1AbCdEfGhIjKlMnOpQrStUvWxYz012345";
    assert_eq!(
        zajil_core::drive::download_link(&format!(
            "https://drive.google.com/file/d/{id}/view?usp=sharing"
        ))
        .as_deref(),
        Some(
            format!("https://drive.usercontent.google.com/download?id={id}&export=download")
                .as_str()
        )
    );
    assert_eq!(
        zajil_core::drive::download_link("https://example.com/file/d/x/view"),
        None
    );
    assert_eq!(zajil_core::drive::download_link("not a link"), None);
}
