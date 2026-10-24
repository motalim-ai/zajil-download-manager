//! The M5 criterion (ROADMAP): real Native Messaging messages sent to
//! `zajil-host`, as the browser sends them (stdin, each message preceded
//! by its length in 4 bytes), on the three systems. A correct message
//! reaches the app; a wrong one, an oversized one, or one with an extra
//! field is refused, and never reaches it.
//!
//! The app is stood in for by a listener on the same private channel
//! (`zajil_platform::ipc`) that records what arrives and answers ok.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use zajil_host::{MAX_REQUEST, Request, Response, read_frame, write_frame};

/// The host, killed and waited for even when the test fails first.
struct Host(Child);

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_host(data_dir: &Path) -> Host {
    let child = Command::new(env!("CARGO_BIN_EXE_zajil-host"))
        // What Chrome passes: the calling extension's origin.
        .arg("chrome-extension://abcdefghijklmnopabcdefghijklmnop/")
        .env("ZAJIL_DATA_DIR", data_dir)
        .env("ZAJIL_NO_LAUNCH", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("zajil-host starts");
    Host(child)
}

/// A stand-in for the app: every request that reaches it is sent on.
fn start_app(data_dir: &Path) -> mpsc::Receiver<Request> {
    let listener = zajil_platform::ipc::Listener::bind(data_dir).expect("the channel opens");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(mut stream) = listener.accept() {
            let Ok(Some(message)) = read_frame(&mut stream, MAX_REQUEST) else {
                continue;
            };
            let request: Request =
                serde_json::from_slice(&message).expect("the host sends requests");
            let _ = tx.send(request);
            let _ = write_frame(&mut stream, &serde_json::to_vec(&Response::ok()).unwrap());
        }
    });
    rx
}

fn send(host: &mut Host, message: &[u8]) {
    let stdin = host.0.stdin.as_mut().unwrap();
    write_frame(stdin, message).unwrap();
}

fn answer(host: &mut Host) -> Response {
    let stdout = host.0.stdout.as_mut().unwrap();
    let message = read_frame(stdout, 1 << 20).unwrap().expect("an answer");
    serde_json::from_slice(&message).unwrap()
}

const GOOD: &str = r#"{"type":"download","protocol_version":1,"url":"https://example.com/files/a.zip","referrer":"https://example.com/","filename":"a.zip","cookies":"sid=1"}"#;

#[test]
fn a_correct_message_reaches_the_app_and_wrong_ones_never_do() {
    let data = tempfile::tempdir().unwrap();
    let arrived = start_app(data.path());
    let mut host = start_host(data.path());

    send(&mut host, GOOD.as_bytes());
    assert_eq!(answer(&mut host), Response::ok());
    assert_eq!(
        arrived
            .recv_timeout(Duration::from_secs(10))
            .expect("it reached the app"),
        Request::Download {
            protocol_version: 1,
            url: "https://example.com/files/a.zip".into(),
            referrer: Some("https://example.com/".into()),
            filename: Some("a.zip".into()),
            cookies: Some("sid=1".into()),
        }
    );

    // A ping is answered by the host itself.
    send(&mut host, br#"{"type":"ping","protocol_version":1}"#);
    assert_eq!(answer(&mut host), Response::ok());

    for wrong in [
        "{not json",
        r#"{"type":"download","protocol_version":1,"url":"https://example.com/a.zip","extra":1}"#,
        r#"{"type":"download","protocol_version":9,"url":"https://example.com/a.zip"}"#,
        r#"{"type":"download","protocol_version":1,"url":"file:///etc/passwd"}"#,
        r#"{"type":"download","protocol_version":1,"url":"https://example.com/a.zip","cookies":"a=1\r\nHost: evil"}"#,
        r#"{"type":"shell","protocol_version":1,"command":"rm"}"#,
    ] {
        send(&mut host, wrong.as_bytes());
        let response = answer(&mut host);
        assert!(!response.ok, "{wrong}: {response:?}");
        assert!(response.error.is_some(), "{wrong}");
    }
    // The connection is still good after refusals, and nothing wrong
    // reached the app.
    send(&mut host, GOOD.as_bytes());
    assert_eq!(answer(&mut host), Response::ok());
    assert!(arrived.recv_timeout(Duration::from_secs(10)).is_ok());
    assert!(
        arrived.try_recv().is_err(),
        "a refused message reached the app"
    );
}

/// An oversized message is refused by its length alone: the host answers
/// why and ends, since what follows cannot be trusted to be a message.
#[test]
fn an_oversized_message_is_refused_and_the_host_ends() {
    let data = tempfile::tempdir().unwrap();
    let arrived = start_app(data.path());
    let mut host = start_host(data.path());
    let stdin = host.0.stdin.as_mut().unwrap();
    stdin.write_all(&(MAX_REQUEST + 1).to_ne_bytes()).unwrap();
    stdin.write_all(GOOD.as_bytes()).unwrap();
    stdin.flush().unwrap();
    let response = answer(&mut host);
    assert!(!response.ok);
    assert!(
        response.error.unwrap().contains("at most"),
        "the limit is said"
    );
    let status = host.0.wait().unwrap();
    assert!(!status.success());
    assert!(arrived.try_recv().is_err());
}

/// Without a running app (and not allowed to start one), a download is
/// refused with a clear reason; the host keeps answering.
#[test]
fn without_the_app_a_download_is_refused_clearly() {
    let data = tempfile::tempdir().unwrap();
    let mut host = start_host(data.path());
    send(&mut host, GOOD.as_bytes());
    let response = answer(&mut host);
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("Zajil is not running"));
    send(&mut host, br#"{"type":"ping","protocol_version":1}"#);
    assert_eq!(answer(&mut host), Response::ok());
}

/// The browser closing the connection ends the host cleanly.
#[test]
fn the_host_ends_when_the_browser_closes() {
    let data = tempfile::tempdir().unwrap();
    let mut host = start_host(data.path());
    drop(host.0.stdin.take());
    let status = host.0.wait().unwrap();
    assert!(status.success());
    let mut rest = Vec::new();
    host.0
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut rest)
        .unwrap();
    assert!(rest.is_empty());
}
