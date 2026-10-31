//! The Android app's engine interface (`Downloads`), on the host, against
//! the local test server: the same Rust path the app takes, without the
//! JVM. What only Android can show (the notification, killing the app) is
//! tested on the emulator (android/README.md).

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use sha2::{Digest, Sha256};
use zajil_android::{Downloads, Listener, Problem};
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
