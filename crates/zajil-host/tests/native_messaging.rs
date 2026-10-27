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

use zajil_host::{Links, MAX_REQUEST, Request, Response, ToExtension, read_frame, write_frame};

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
            instance: None,
            store: None,
            ticket: None,
            private: false,
        }
    );

    // A ping is answered by the host itself, with Zajil's language: Arabic
    // while there are no settings yet, then what the settings say.
    send(&mut host, br#"{"type":"ping","protocol_version":1}"#);
    assert_eq!(answer(&mut host), Response::pong(Some("ar")));
    zajil_core::settings::SettingsStore::new(data.path())
        .update(|s| {
            s.language = zajil_core::settings::Language::En;
            Ok(())
        })
        .unwrap();
    send(&mut host, br#"{"type":"ping","protocol_version":1}"#);
    assert_eq!(answer(&mut host), Response::pong(Some("en")));

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
    assert_eq!(answer(&mut host), Response::pong(Some("ar")));
}

/// #60: only another start of Zajil asks for its window; from the
/// browser, `zajil-host` refuses it, and the app is never asked.
#[test]
fn the_browser_cannot_ask_for_the_window() {
    let data = tempfile::tempdir().unwrap();
    let mut host = start_host(data.path());
    send(&mut host, br#"{"type":"show_window","protocol_version":1}"#);
    let response = answer(&mut host);
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("not from the browser"));
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

/// Review of #62: an app that takes the connection and never answers
/// cannot hold the browser's call: the host answers for it within its
/// time limit, and keeps answering.
#[test]
fn an_app_that_never_answers_does_not_hold_the_browser() {
    let data = tempfile::tempdir().unwrap();
    let listener = zajil_platform::ipc::Listener::bind(data.path()).unwrap();
    let held = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let keep = held.clone();
    std::thread::spawn(move || {
        while let Ok(stream) = listener.accept() {
            keep.lock().unwrap().push(stream); // taken, never answered
        }
    });
    let mut host = start_host(data.path());
    let started = std::time::Instant::now();
    send(&mut host, GOOD.as_bytes());
    let response = answer(&mut host);
    let took = started.elapsed();
    assert!(!response.ok);
    assert!(
        response.error.unwrap().contains("did not answer"),
        "the reason is said"
    );
    assert!(
        took < zajil_host::CHANNEL_TIMEOUT + Duration::from_secs(5),
        "took {took:?}"
    );
    send(&mut host, br#"{"type":"ping","protocol_version":1}"#);
    assert_eq!(answer(&mut host), Response::pong(Some("ar")));
}

/// M5, after a restart: the extension's link (`runtime.connectNative`).
/// The host waits for an app that is not running yet, and never starts
/// one; then the app's questions reach the extension and its answers come
/// back; an answer the host refuses, or none, comes back as no answer
/// within LINK_ANSWER; and when the browser closes the link, the app
/// knows it is gone.
#[test]
fn the_link_carries_zajil_s_questions_and_the_extension_s_answers() {
    let data = tempfile::tempdir().unwrap();
    let mut host = start_host(data.path());
    send(
        &mut host,
        br#"{"type":"link","protocol_version":1,"instance":"3f2b8c1e-aaaa"}"#,
    );
    std::thread::sleep(Duration::from_millis(500));
    // The app opens after the browser.
    let listener = zajil_platform::ipc::Listener::bind(data.path()).unwrap();
    let links = Links::default();
    let (linked_tx, linked) = mpsc::channel();
    links.on_link(move |instance| {
        let _ = linked_tx.send(instance.to_owned());
    });
    {
        let links = links.clone();
        std::thread::spawn(move || zajil_host::serve(listener, |_| Response::ok(), links));
    }
    assert_eq!(
        linked.recv_timeout(Duration::from_secs(10)).unwrap(),
        "3f2b8c1e-aaaa"
    );
    assert!(links.is_linked("3f2b8c1e-aaaa"));

    let ask = |links: &Links| {
        let links = links.clone();
        std::thread::spawn(move || {
            links.ask(
                "3f2b8c1e-aaaa",
                "https://example.com/a.zip",
                Some("firefox-container-1"),
                "ticket-1",
                Duration::from_secs(20),
            )
        })
    };
    let question = |host: &mut Host| {
        let stdout = host.0.stdout.as_mut().unwrap();
        let message = read_frame(stdout, 1 << 20).unwrap().expect("a question");
        ToExtension::parse(&message).unwrap()
    };

    let asking = ask(&links);
    let ToExtension::CookiesWanted {
        ask: id,
        url,
        store,
        ticket,
        ..
    } = question(&mut host)
    else {
        panic!("not a cookie question")
    };
    assert_eq!(ticket, "ticket-1");
    assert_eq!(url, "https://example.com/a.zip");
    assert_eq!(store.as_deref(), Some("firefox-container-1"));
    send(
        &mut host,
        format!(r#"{{"type":"cookies","protocol_version":1,"ask":{id},"cookies":"sid=2"}}"#)
            .as_bytes(),
    );
    assert_eq!(asking.join().unwrap(), Some(Some("sid=2".into())));

    // No permission: answered, without cookies.
    let asking = ask(&links);
    let id = question(&mut host).ask();
    send(
        &mut host,
        format!(r#"{{"type":"cookies","protocol_version":1,"ask":{id},"cookies":null}}"#)
            .as_bytes(),
    );
    assert_eq!(asking.join().unwrap(), Some(None));

    // An answer with a line break is refused by the host: no answer.
    let asking = ask(&links);
    let id = question(&mut host).ask();
    send(
        &mut host,
        format!(
            r#"{{"type":"cookies","protocol_version":1,"ask":{id},"cookies":"a=1\r\nHost: x"}}"#
        )
        .as_bytes(),
    );
    let started = std::time::Instant::now();
    assert_eq!(asking.join().unwrap(), None);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );

    // An answer to another question (a late one) is not taken: the host
    // waits for this one's, and says none came (review of #80).
    let asking = ask(&links);
    let id = question(&mut host).ask();
    send(
        &mut host,
        format!(
            r#"{{"type":"cookies","protocol_version":1,"ask":{},"cookies":"sid=OTHER"}}"#,
            id + 1000
        )
        .as_bytes(),
    );
    assert_eq!(asking.join().unwrap(), None);

    // Which downloads are still open: passed on, and answered.
    let keeping = {
        let links = links.clone();
        std::thread::spawn(move || {
            links.keep(
                "3f2b8c1e-aaaa",
                vec!["ticket-2".into()],
                Duration::from_secs(20),
            )
        })
    };
    let ToExtension::Keep {
        ask: id, tickets, ..
    } = question(&mut host)
    else {
        panic!("not a keep")
    };
    assert_eq!(tickets, ["ticket-2"]);
    send(
        &mut host,
        format!(r#"{{"type":"cookies","protocol_version":1,"ask":{id},"cookies":null}}"#)
            .as_bytes(),
    );
    assert!(keeping.join().unwrap());

    // The browser closes the link: the host ends, and the app knows.
    drop(host.0.stdin.take());
    let _ = host.0.wait();
    assert_eq!(ask(&links).join().unwrap(), None);
    assert!(!links.is_linked("3f2b8c1e-aaaa"));
}

/// An answer to another question is never taken for this one (review of
/// #80: a late answer about one site's cookies would have gone to another
/// download), on the app's side; the host's side is checked above.
#[test]
fn an_answer_to_another_question_is_not_taken() {
    let data = tempfile::tempdir().unwrap();
    let listener = zajil_platform::ipc::Listener::bind(data.path()).unwrap();
    let links = Links::default();
    {
        let links = links.clone();
        std::thread::spawn(move || zajil_host::serve(listener, |_| Response::ok(), links));
    }
    let path = data.path().to_path_buf();
    let host = std::thread::spawn(move || {
        let mut app = zajil_platform::ipc::connect(&path, Duration::from_secs(5)).unwrap();
        write_frame(
            &mut app,
            br#"{"type":"link","protocol_version":1,"instance":"one"}"#,
        )
        .unwrap();
        let question = read_frame(&mut app, MAX_REQUEST).unwrap().unwrap();
        let ask = ToExtension::parse(&question).unwrap().ask();
        let wrong = zajil_host::FromLink::Cookies {
            protocol_version: 1,
            ask: ask + 1,
            cookies: Some("sid=OTHER-SITE".into()),
        };
        zajil_host::write_json(&mut app, &wrong).unwrap();
        app
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !links.is_linked("one") {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let answer = links.ask(
        "one",
        "https://example.com/a.zip",
        None,
        "ticket-1",
        Duration::from_secs(10),
    );
    assert_eq!(answer, None);
    let _app = host.join().unwrap();
}
