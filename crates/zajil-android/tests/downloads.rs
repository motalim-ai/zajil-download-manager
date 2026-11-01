//! The Android app's engine interface (`Downloads`), on the host, against
//! the local test server: the same Rust path the app takes, without the
//! JVM. What only Android can show (the notification, killing the app) is
//! tested on the emulator (android/README.md).

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use sha2::{Digest, Sha256};
use zajil_android::{Downloads, EngineError, HostCheck, Listener, Problem, Tuning};
use zajil_core::{Engine, Options};
use zajil_testserver::{FileSpec, TestServer, Tls, test_ca_der};

#[derive(Debug, Clone, PartialEq)]
enum Heard {
    Started { state_file: String, from: u64 },
    Progress { done: u64 },
    Finished { file: String, bytes: u64 },
    Paused { state_file: Option<String> },
    Failed { problem: Problem, detail: String },
}

struct Ears(Mutex<Sender<(i64, Heard)>>);

impl Ears {
    fn new() -> (Arc<Self>, Receiver<(i64, Heard)>) {
        let (tx, rx) = channel();
        (Arc::new(Ears(Mutex::new(tx))), rx)
    }

    fn hear(&self, id: i64, heard: Heard) {
        let _ = self.0.lock().unwrap().send((id, heard));
    }
}

impl Listener for Ears {
    fn started(&self, id: i64, _file: String, state_file: String, _total: Option<u64>, from: u64) {
        self.hear(id, Heard::Started { state_file, from });
    }

    fn progress(&self, id: i64, done: u64, _total: Option<u64>) {
        self.hear(id, Heard::Progress { done });
    }

    fn finished(&self, id: i64, file: String, bytes: u64) {
        self.hear(id, Heard::Finished { file, bytes });
    }

    fn paused(&self, id: i64, state_file: Option<String>) {
        self.hear(id, Heard::Paused { state_file });
    }

    fn failed(&self, id: i64, problem: Problem, detail: String, _state_file: Option<String>) {
        self.hear(id, Heard::Failed { problem, detail });
    }
}

/// The first event `wanted` picks out, within a generous time.
fn wait_for<T>(rx: &Receiver<(i64, Heard)>, mut wanted: impl FnMut(i64, &Heard) -> Option<T>) -> T {
    loop {
        let (id, heard) = rx
            .recv_timeout(Duration::from_secs(60))
            .expect("no event from the engine in 60 s");
        if let Some(found) = wanted(id, &heard) {
            return found;
        }
    }
}

fn finished(rx: &Receiver<(i64, Heard)>, want: i64) -> (String, u64) {
    wait_for(rx, |id, heard| match heard {
        Heard::Finished { file, bytes } if id == want => Some((file.clone(), *bytes)),
        Heard::Failed { detail, .. } if id == want => panic!("download {id} failed: {detail}"),
        _ => None,
    })
}

fn failed(rx: &Receiver<(i64, Heard)>, want: i64) -> Problem {
    wait_for(rx, |id, heard| match heard {
        Heard::Failed { problem, .. } if id == want => Some(*problem),
        Heard::Finished { .. } if id == want => panic!("download {id} should have failed"),
        _ => None,
    })
}

fn random_body(len: usize) -> Vec<u8> {
    let mut body = vec![0u8; len];
    getrandom::fill(&mut body).unwrap();
    body
}

fn sha256(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

fn dir_text(dir: &tempfile::TempDir) -> String {
    dir.path().to_string_lossy().into_owned()
}

#[test]
fn downloads_a_file_and_the_sha256_matches() {
    let body = random_body(3 * 1024 * 1024);
    let server = TestServer::http();
    server.put("/file.bin", FileSpec::new(body.clone()));
    let dir = tempfile::tempdir().unwrap();
    let (ears, rx) = Ears::new();
    let downloads = Downloads::new(ears).unwrap();

    downloads.start(1, server.url("/file.bin"), dir_text(&dir));
    let (file, bytes) = finished(&rx, 1);

    assert_eq!(bytes, body.len() as u64);
    assert_eq!(sha256(&std::fs::read(file).unwrap()), sha256(&body));
    assert!(!downloads.is_running(1));
}

#[test]
fn pause_then_resume_asks_only_for_the_rest() {
    let size = 8 * 1024 * 1024;
    let body = random_body(size);
    let server = TestServer::http();
    // Slow enough to pause in the middle: about 8 s in all.
    server.put(
        "/slow.bin",
        FileSpec::new(body.clone()).throttle(1024 * 1024),
    );
    let dir = tempfile::tempdir().unwrap();
    let (ears, rx) = Ears::new();
    let downloads = Downloads::new(ears).unwrap();

    downloads.start(7, server.url("/slow.bin"), dir_text(&dir));
    let state_file = wait_for(&rx, |_, heard| match heard {
        Heard::Started { state_file, .. } => Some(state_file.clone()),
        _ => None,
    });
    wait_for(&rx, |_, heard| match heard {
        Heard::Progress { done } if *done >= size as u64 / 3 => Some(()),
        _ => None,
    });
    downloads.pause(7);
    let paused = wait_for(&rx, |_, heard| match heard {
        Heard::Paused { state_file } => Some(state_file.clone()),
        Heard::Finished { .. } => panic!("finished before the pause"),
        _ => None,
    });
    assert_eq!(paused.as_deref(), Some(state_file.as_str()));
    assert!(!downloads.is_running(7));

    downloads.resume(7, state_file);
    let from = wait_for(&rx, |_, heard| match heard {
        Heard::Started { from, .. } => Some(*from),
        _ => None,
    });
    assert!(from > 0, "the resume started over from byte 0");
    let (file, _) = finished(&rx, 7);

    assert_eq!(sha256(&std::fs::read(file).unwrap()), sha256(&body));
    // Bytes written but not yet synced at the pause may be asked for again,
    // never the whole first part.
    let served: u64 = server.requests().iter().map(|r| r.bytes_sent).sum();
    assert!(
        served <= size as u64 + 1024 * 1024,
        "the server sent {served} bytes for a file of {size}"
    );
}

#[test]
fn starting_a_running_download_again_does_nothing() {
    let server = TestServer::http();
    server.put(
        "/slow.bin",
        FileSpec::new(random_body(2 * 1024 * 1024)).throttle(512 * 1024),
    );
    let dir = tempfile::tempdir().unwrap();
    let (ears, rx) = Ears::new();
    let downloads = Downloads::new(ears).unwrap();

    downloads.start(3, server.url("/slow.bin"), dir_text(&dir));
    downloads.start(3, server.url("/slow.bin"), dir_text(&dir));
    assert_eq!(downloads.running(), 1);
    finished(&rx, 3);
    // Pausing what no longer runs is harmless.
    downloads.pause(3);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn https_with_a_trusted_certificate_downloads() {
    let body = random_body(256 * 1024);
    let server = TestServer::https(Tls::trusted(&["localhost"]), "localhost");
    server.put("/file.bin", FileSpec::new(body.clone()));
    let dir = tempfile::tempdir().unwrap();
    let (ears, rx) = Ears::new();
    let engine = Engine::with_test_root(Options::default(), &test_ca_der());
    let downloads = Downloads::with_engine(ears, engine).unwrap();

    downloads.start(1, server.url("/file.bin"), dir_text(&dir));
    let (file, _) = finished(&rx, 1);

    assert_eq!(sha256(&std::fs::read(file).unwrap()), sha256(&body));
}

#[test]
fn https_with_a_self_signed_certificate_fails_on_the_certificate() {
    let server = TestServer::https_self_signed(&["localhost"], "localhost");
    server.put("/file.bin", FileSpec::new(random_body(1024)));
    let dir = tempfile::tempdir().unwrap();
    let (ears, rx) = Ears::new();
    // Even an engine that trusts the test CA refuses it: the same path as
    // the success above, only the certificate differs.
    let engine = Engine::with_test_root(Options::default(), &test_ca_der());
    let downloads = Downloads::with_engine(ears, engine).unwrap();

    downloads.start(1, server.url("/file.bin"), dir_text(&dir));

    assert_eq!(failed(&rx, 1), Problem::Certificate);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn https_with_an_expired_certificate_fails_on_the_certificate() {
    let server = TestServer::https(Tls::expired(&["localhost"]), "localhost");
    server.put("/file.bin", FileSpec::new(random_body(1024)));
    let dir = tempfile::tempdir().unwrap();
    let (ears, rx) = Ears::new();
    let engine = Engine::with_test_root(Options::default(), &test_ca_der());
    let downloads = Downloads::with_engine(ears, engine).unwrap();

    downloads.start(1, server.url("/file.bin"), dir_text(&dir));

    assert_eq!(failed(&rx, 1), Problem::Certificate);
}

#[test]
fn a_link_zajil_cannot_download_is_a_bad_link() {
    let dir = tempfile::tempdir().unwrap();
    let (ears, rx) = Ears::new();
    let downloads = Downloads::new(ears).unwrap();

    downloads.start(1, "ftp://example.invalid/x".to_owned(), dir_text(&dir));

    assert_eq!(failed(&rx, 1), Problem::BadLink);
}

#[test]
fn a_tuned_start_takes_the_name_it_is_given() {
    let body = random_body(512 * 1024);
    let server = TestServer::http();
    server.put("/videoplayback", FileSpec::new(body.clone()));
    let dir = tempfile::tempdir().unwrap();
    let (ears, rx) = Ears::new();
    let downloads = Downloads::new(ears).unwrap();
    let tuning = Tuning {
        name: Some("video.mp4".to_owned()),
        user_agent: Some("Zajil-test".to_owned()),
        connections: Some(4),
        request_size: Some(64 * 1024),
        ..Tuning::default()
    };

    downloads.start_tuned(1, server.url("/videoplayback"), dir_text(&dir), tuning);
    let (file, _) = finished(&rx, 1);

    assert!(file.ends_with("video.mp4"), "{file}");
    assert_eq!(sha256(&std::fs::read(file).unwrap()), sha256(&body));
}

#[test]
fn a_new_link_to_the_same_file_keeps_what_was_downloaded() {
    let size = 8 * 1024 * 1024;
    let body = random_body(size);
    let server = TestServer::http();
    server.put(
        "/old",
        FileSpec::new(body.clone())
            .throttle(1024 * 1024)
            .etag(Some("\"same\"")),
    );
    server.put("/new", FileSpec::new(body.clone()).etag(Some("\"same\"")));
    let dir = tempfile::tempdir().unwrap();
    let (ears, rx) = Ears::new();
    let downloads = Downloads::new(ears).unwrap();

    downloads.start(5, server.url("/old"), dir_text(&dir));
    wait_for(&rx, |_, heard| match heard {
        Heard::Progress { done } if *done >= size as u64 / 4 => Some(()),
        _ => None,
    });
    downloads.pause(5);
    let state_file = wait_for(&rx, |_, heard| match heard {
        Heard::Paused { state_file } => state_file.clone(),
        _ => None,
    });

    downloads
        .refresh_link(state_file.clone(), server.url("/new"))
        .unwrap();
    downloads.resume(5, state_file);
    let (file, _) = finished(&rx, 5);

    assert_eq!(sha256(&std::fs::read(file).unwrap()), sha256(&body));
    // The rest came from the new link only.
    let new_bytes: u64 = server
        .requests()
        .iter()
        .filter(|r| r.path.ends_with("/new"))
        .map(|r| r.bytes_sent)
        .sum();
    assert!(new_bytes < size as u64, "the new link sent the whole file");
}

#[test]
fn probe_tells_the_name_the_size_and_that_it_resumes() {
    let body = random_body(300 * 1024);
    let server = TestServer::http();
    server.put(
        "/get?id=7",
        FileSpec::new(body.clone()).content_disposition("attachment; filename=\"تقرير.pdf\""),
    );
    let (ears, _rx) = Ears::new();
    let downloads = Downloads::new(ears).unwrap();

    let info = downloads.probe(server.url("/get?id=7")).unwrap();

    assert_eq!(info.name, "تقرير.pdf");
    assert_eq!(info.size, Some(body.len() as u64));
    assert!(info.resumable);
    assert!(!info.is_page);
}

#[test]
fn probe_says_when_a_file_cannot_resume() {
    let server = TestServer::http();
    server.put(
        "/once.bin",
        FileSpec::new(random_body(64 * 1024)).without_ranges(),
    );
    let (ears, _rx) = Ears::new();
    let downloads = Downloads::new(ears).unwrap();

    let info = downloads.probe(server.url("/once.bin")).unwrap();

    assert!(!info.resumable);
}

#[test]
fn probe_says_when_a_link_leads_to_a_web_page() {
    let server = TestServer::http();
    server.put(
        "/login",
        FileSpec::new(b"<html></html>".to_vec()).content_type("text/html; charset=utf-8"),
    );
    let (ears, _rx) = Ears::new();
    let downloads = Downloads::new(ears).unwrap();

    let info = downloads.probe(server.url("/login")).unwrap();

    assert!(info.is_page);
    assert_eq!(info.content_type.as_deref(), Some("text/html"));
}

#[test]
fn probe_of_a_link_zajil_cannot_download_is_a_bad_link() {
    let (ears, _rx) = Ears::new();
    let downloads = Downloads::new(ears).unwrap();

    let err = downloads
        .probe("ftp://example.invalid/x".to_owned())
        .unwrap_err();

    assert!(
        matches!(
            err,
            zajil_android::EngineError::Probe {
                problem: Problem::BadLink,
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn the_speed_limit_slows_every_download_and_can_be_lifted() {
    // 512 KiB at 128 KiB/s: about four seconds; without the limit, a
    // fraction of one on the local server.
    let body = random_body(512 * 1024);
    let server = TestServer::http();
    server.put("/limited.bin", FileSpec::new(body.clone()));
    let dir = tempfile::tempdir().unwrap();
    let (ears, rx) = Ears::new();
    let downloads = Downloads::new(ears).unwrap();

    downloads.set_speed_limit(Some(128 * 1024));
    let began = std::time::Instant::now();
    downloads.start(1, server.url("/limited.bin"), dir_text(&dir));
    let (file, _) = finished(&rx, 1);
    let limited = began.elapsed();
    assert!(limited >= Duration::from_secs(2), "took {limited:?}");
    assert_eq!(sha256(&std::fs::read(file).unwrap()), sha256(&body));

    downloads.set_speed_limit(None);
    let dir = tempfile::tempdir().unwrap();
    let began = std::time::Instant::now();
    downloads.start(2, server.url("/limited.bin"), dir_text(&dir));
    finished(&rx, 2);
    assert!(
        began.elapsed() < limited / 2,
        "took {:?} without a limit",
        began.elapsed()
    );
}

#[test]
fn the_browsers_cookies_and_page_go_with_a_tuned_download() {
    let body = random_body(256 * 1024);
    let server = TestServer::http();
    server.put("/member/clip.mp4", FileSpec::new(body.clone()));
    let dir = tempfile::tempdir().unwrap();
    let (ears, rx) = Ears::new();
    let downloads = Downloads::new(ears).unwrap();
    let url = server.url("/member/clip.mp4");
    let page = server.url("/member/page?id=1");
    let tuning = Tuning {
        cookies: Some("session=abc".to_owned()),
        cookie_url: Some(url.clone()),
        referrer: Some(page.clone()),
        ..Tuning::default()
    };

    downloads.start_tuned(1, url, dir_text(&dir), tuning);
    finished(&rx, 1);

    let requests = server.requests();
    assert!(!requests.is_empty());
    for request in requests.iter().filter(|r| r.path.ends_with("clip.mp4")) {
        assert_eq!(
            request.cookie.as_deref(),
            Some("session=abc"),
            "{request:?}"
        );
        // The same origin: the whole page, as the browsers send it.
        assert_eq!(
            request.referer.as_deref(),
            Some(page.as_str()),
            "{request:?}"
        );
    }
}

#[test]
fn the_browsers_cookies_never_follow_a_redirect_to_another_host() {
    let body = random_body(64 * 1024);
    let server = TestServer::http();
    server.put("/cdn/clip.mp4", FileSpec::new(body));
    // Another host for the same server: «localhost» instead of 127.0.0.1.
    let elsewhere = server
        .url("/cdn/clip.mp4")
        .replace("127.0.0.1", "localhost");
    server.redirect("/go", &elsewhere, 302);
    let dir = tempfile::tempdir().unwrap();
    let (ears, rx) = Ears::new();
    let downloads = Downloads::new(ears).unwrap();
    let url = server.url("/go");
    let tuning = Tuning {
        cookies: Some("session=abc".to_owned()),
        cookie_url: Some(url.clone()),
        referrer: Some(server.url("/page")),
        ..Tuning::default()
    };

    downloads.start_tuned(1, url, dir_text(&dir), tuning);
    finished(&rx, 1);

    for request in server
        .requests()
        .iter()
        .filter(|r| r.path.ends_with("clip.mp4"))
    {
        assert_eq!(request.cookie, None, "{request:?}");
    }
}

#[test]
fn a_tuned_probe_sends_the_browsers_cookies() {
    let server = TestServer::http();
    server.put("/member/f.bin", FileSpec::new(random_body(4096)));
    let (ears, _rx) = Ears::new();
    let downloads = Downloads::new(ears).unwrap();
    let url = server.url("/member/f.bin");
    let tuning = Tuning {
        cookies: Some("session=abc".to_owned()),
        cookie_url: Some(url.clone()),
        ..Tuning::default()
    };

    downloads.probe_tuned(url, tuning).unwrap();

    assert_eq!(server.requests()[0].cookie.as_deref(), Some("session=abc"));
}

/// The app's lists of harmful sites, in a test: these hosts.
struct Refuse(Vec<&'static str>);

impl HostCheck for Refuse {
    fn refused(&self, host: String) -> bool {
        self.0.contains(&host.as_str())
    }
}

#[test]
fn a_harmful_host_is_never_asked_nor_followed_to() {
    let server = TestServer::http();
    server.put("/cdn/clip.mp4", FileSpec::new(random_body(4096)));
    // A redirect to another host of the same server: «localhost».
    let elsewhere = server
        .url("/cdn/clip.mp4")
        .replace("127.0.0.1", "localhost");
    server.redirect("/go", &elsewhere, 302);
    let dir = tempfile::tempdir().unwrap();
    let (ears, rx) = Ears::new();
    let downloads = Downloads::new(ears).unwrap();
    downloads.set_host_check(Some(Arc::new(Refuse(vec!["localhost"]))));

    // Through a redirect: the first host answers, the harmful one is never asked.
    downloads.start(1, server.url("/go"), dir_text(&dir));
    assert_eq!(failed(&rx, 1), Problem::Refused);
    assert!(
        server
            .requests()
            .iter()
            .all(|r| !r.path.ends_with("clip.mp4")),
        "{:?}",
        server.requests()
    );
    // Directly, and in a probe: nothing is asked at all.
    downloads.start(2, elsewhere.clone(), dir_text(&dir));
    assert_eq!(failed(&rx, 2), Problem::Refused);
    assert!(matches!(
        downloads.probe(elsewhere.clone()),
        Err(EngineError::Probe { .. })
    ));
    assert_eq!(
        server.requests().len(),
        1,
        "only /go was asked: {:?}",
        server.requests()
    );
    // No list: it downloads.
    downloads.set_host_check(None);
    downloads.start(3, elsewhere, dir_text(&dir));
    finished(&rx, 3);
}
