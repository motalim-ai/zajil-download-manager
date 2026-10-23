//! `zajil queue run --shutdown-after` and `--sleep-after` (M3). No test
//! here lets the countdown end: the one that reaches it kills `zajil`
//! during a countdown of an hour. And every `zajil` of the tests runs with
//! `ZAJIL_NO_POWER_ACTIONS` set (`tests/common/mod.rs`), so even a mistake
//! would only be refused.

mod common;

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Output, Stdio};
use std::time::Duration;

use common::zajil_command;
use zajil_testserver::{FileSpec, TestServer, random_bytes};

fn zajil(data: &Path, args: &[&str]) -> Output {
    zajil_command()
        .arg("--data-dir")
        .arg(data)
        .args(args)
        .output()
        .unwrap()
}

fn printed(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn add(data: &Path, downloads: &Path, url: &str) {
    let out = zajil(
        data,
        &["queue", "add", "--dir", downloads.to_str().unwrap(), url],
    );
    assert!(out.status.success(), "{}", printed(&out));
}

/// The queue finishes, so a clear countdown starts, saying how to cancel
/// it. The test stops `zajil` there, long before the hour is over.
#[test]
fn a_finished_queue_starts_a_countdown_that_says_how_to_cancel_it() {
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(random_bytes(50_000, 950)));
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    add(data.path(), downloads.path(), &server.url("/f.bin"));

    let mut child = zajil_command()
        .arg("--data-dir")
        .arg(data.path())
        .args([
            "queue",
            "run",
            "--shutdown-after",
            "main",
            "--countdown",
            "3600",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Lines arrive through a channel, so a missing line fails the test
    // after a minute instead of hanging it.
    let (send, lines) = std::sync::mpsc::channel();
    let stderr = child.stderr.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            if send.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let mut seen = Vec::new();
    loop {
        match lines.recv_timeout(Duration::from_secs(60)) {
            Ok(line) => {
                let counting = line.contains("will shut down in 3600 s");
                seen.push(line);
                if counting {
                    break;
                }
            }
            Err(_) => {
                let _ = child.kill();
                panic!("no countdown: {seen:?}");
            }
        }
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let shown = seen.join("\n");
    assert!(
        shown.contains("the computer will shut down when queue main has finished"),
        "{shown}"
    );
    assert!(shown.contains("[1] saved"), "{shown}");
    assert!(
        shown.contains(
            "queue main has finished: the computer will shut down in 3600 s; press Ctrl+C to cancel"
        ),
        "{shown}"
    );
}

/// A download of the queue failed: no countdown, and the computer stays
/// on.
#[test]
fn a_queue_that_did_not_finish_keeps_the_computer_on() {
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(random_bytes(50_000, 951)));
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    add(data.path(), downloads.path(), &server.url("/f.bin"));
    add(data.path(), downloads.path(), &server.url("/missing.bin"));

    let out = zajil(
        data.path(),
        &["--tries", "1", "queue", "run", "--sleep-after", "main"],
    );
    let shown = printed(&out);
    assert_eq!(out.status.code(), Some(1), "{shown}");
    assert!(
        shown.contains("queue main did not finish (1 download(s) not done"),
        "{shown}"
    );
    assert!(shown.contains("the computer stays on"), "{shown}");
    assert!(!shown.contains("Ctrl+C to cancel"), "{shown}");
}

#[test]
fn the_power_options_are_checked() {
    let data = tempfile::tempdir().unwrap();
    for (args, code, says) in [
        (
            &[
                "queue",
                "run",
                "--shutdown-after",
                "main",
                "--sleep-after",
                "main",
            ][..],
            2,
            "cannot be used with",
        ),
        (
            &[
                "queue",
                "run",
                "--shutdown-after",
                "main",
                "--countdown",
                "5",
            ],
            2,
            "5",
        ),
        (
            &["queue", "run", "--shutdown-after", "nosuch"],
            1,
            "there is no queue \"nosuch\"",
        ),
    ] {
        let out = zajil(data.path(), args);
        let shown = printed(&out);
        assert_eq!(out.status.code(), Some(code), "{args:?}: {shown}");
        assert!(shown.contains(says), "{args:?}: {shown}");
    }
}

/// Without the options, a run never mentions the computer.
#[test]
fn without_being_asked_zajil_never_touches_the_computer() {
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(random_bytes(50_000, 952)));
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    add(data.path(), downloads.path(), &server.url("/f.bin"));
    let out = zajil(data.path(), &["queue", "run"]);
    let shown = printed(&out);
    assert!(out.status.success(), "{shown}");
    assert!(!shown.contains("computer"), "{shown}");
}
