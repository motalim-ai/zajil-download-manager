//! Retrying the first request and single-connection downloads (M3, owner's
//! decision on #28): the policy of M2 (`backoff.rs`), applied where M1 gave
//! up at once.

use std::fs;
use std::time::{Duration, Instant};

use zajil_core::state::DownloadState;
use zajil_core::{Engine, Error, Event, Options, RestartReason};
use zajil_testserver::{Failure, FileSpec, TestServer, random_bytes, sha256_hex};

fn engine() -> Engine {
    Engine::new(Options {
        idle_timeout: Duration::from_secs(5),
        ..Options::default()
    })
    .unwrap()
}

fn failure(status: u16, retry_after: Option<u64>) -> Failure {
    Failure {
        status,
        retry_after,
    }
}

fn retries(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, Event::Retrying { .. }))
        .count()
}

fn restarts(events: &[Event]) -> Vec<RestartReason> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Restarted { reason } => Some(*reason),
            _ => None,
        })
        .collect()
}

/// M3 success criterion: a server that does not support ranges cuts the
/// connection twice and answers `429` with `Retry-After` once. The download
/// completes, the SHA-256 matches, and nothing is asked before the
/// `Retry-After` is over.
#[tokio::test]
async fn a_server_without_ranges_that_cuts_twice_and_answers_429_once_completes() {
    let server = TestServer::http();
    let data = random_bytes(5_000_000, 300);
    // GET 1: 429 with Retry-After: 2. GETs 2 and 3: cut after 1.5 MB.
    // GET 4: the whole file.
    server.put(
        "/f.bin",
        FileSpec::new(data.clone())
            .without_ranges()
            .fail_first(vec![failure(429, Some(2))])
            .cut_after(1_500_000, 3),
    );
    let dir = tempfile::tempdir().unwrap();
    let mut events = Vec::new();

    let done = engine()
        .download(&server.url("/f.bin"), dir.path(), &mut |e| events.push(e))
        .await
        .unwrap();

    assert_eq!(
        sha256_hex(&fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );
    let gets = server.gets("/f.bin");
    let statuses: Vec<u16> = gets.iter().map(|r| r.status).collect();
    assert_eq!(statuses, [429, 200, 200, 200], "{gets:?}");
    let waited = gets[1].at - gets[0].at;
    assert!(waited >= Duration::from_millis(1900), "{waited:?}");
    assert!(events.iter().any(|e| matches!(
        e,
        Event::ServerBusy {
            status: 429,
            wait: Some(w),
            ..
        } if *w == Duration::from_secs(2)
    )));
    // Each cut starts over from byte 0, from the whole-file answer: the
    // server cannot continue from the middle.
    assert_eq!(
        restarts(&events),
        [RestartReason::RangesIgnored, RestartReason::RangesIgnored]
    );
    assert_eq!(retries(&events), 3);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn the_first_request_is_tried_again_after_server_errors() {
    let server = TestServer::http();
    let data = random_bytes(200_000, 301);
    server.put(
        "/f.bin",
        FileSpec::new(data.clone()).fail_first(vec![
            failure(503, None),
            failure(500, None),
            failure(408, None),
        ]),
    );
    let dir = tempfile::tempdir().unwrap();
    let mut events = Vec::new();

    let done = engine()
        .download(&server.url("/f.bin"), dir.path(), &mut |e| events.push(e))
        .await
        .unwrap();

    assert_eq!(fs::read(&done.file).unwrap(), data);
    assert_eq!(server.gets("/f.bin").len(), 4);
    assert_eq!(retries(&events), 3);
}

#[tokio::test]
async fn a_cut_single_connection_continues_where_it_stopped() {
    let server = TestServer::http();
    let data = random_bytes(3_000_000, 302);
    server.put(
        "/f.bin",
        FileSpec::new(data.clone()).cut_after(1_000_000, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let mut events = Vec::new();
    let one = Engine::new(Options {
        connections: 1,
        ..Options::default()
    })
    .unwrap();

    let done = one
        .download(&server.url("/f.bin"), dir.path(), &mut |e| events.push(e))
        .await
        .unwrap();

    assert_eq!(
        sha256_hex(&fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );
    let gets = server.gets("/f.bin");
    assert_eq!(gets.len(), 2, "{gets:?}");
    assert_eq!(gets[1].status, 206);
    // The second request asks only for what is not on disk.
    let from: u64 = gets[1]
        .range
        .as_deref()
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.strip_suffix('-'))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{gets:?}"));
    assert!(from > 0 && from <= 1_000_000, "{from}");
    assert!(gets[1].if_range.is_some(), "{gets:?}");
    assert!(restarts(&events).is_empty(), "{events:?}");
    assert_eq!(retries(&events), 1);
}

#[tokio::test]
async fn a_server_that_always_breaks_at_the_same_place_is_given_up() {
    // Without ranges every try starts from byte 0 and breaks at the same
    // byte: no progress, so the download stops after five tries instead
    // of trying forever, and can be resumed later.
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(1_000_000, 303))
            .without_ranges()
            .cut_after(300_000, 100),
    );
    let dir = tempfile::tempdir().unwrap();

    let err = engine()
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();

    let Error::Interrupted { state_file, reason } = err else {
        panic!("{err}");
    };
    assert!(reason.contains("gave up after 5 tries"), "{reason}");
    assert_eq!(server.gets("/f.bin").len(), 5);
    DownloadState::load(&state_file).unwrap();
}

#[tokio::test]
async fn a_server_error_on_every_try_gives_up_after_five() {
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(1_000, 304)).fail_first(vec![failure(503, None); 10]),
    );
    let dir = tempfile::tempdir().unwrap();

    let err = engine()
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();

    assert!(
        matches!(err, Error::Status(s) if s.as_u16() == 503),
        "{err}"
    );
    assert_eq!(server.gets("/f.bin").len(), 5);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn one_try_means_no_retry() {
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(1_000, 305)).fail_first(vec![failure(503, None)]),
    );
    let dir = tempfile::tempdir().unwrap();
    let once = Engine::new(Options {
        max_failures: 1,
        ..Options::default()
    })
    .unwrap();

    let err = once
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();

    assert!(
        matches!(err, Error::Status(s) if s.as_u16() == 503),
        "{err}"
    );
    assert_eq!(server.gets("/f.bin").len(), 1);
}

#[tokio::test]
async fn a_retry_after_longer_than_ten_minutes_is_not_waited_for() {
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(1_000, 306)).fail_first(vec![failure(429, Some(3600))]),
    );
    let dir = tempfile::tempdir().unwrap();
    let started = Instant::now();

    let err = engine()
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();

    assert!(
        matches!(err, Error::ServerBusy { wait } if wait == Duration::from_secs(3600)),
        "{err}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(server.gets("/f.bin").len(), 1);
}

#[tokio::test]
async fn a_refusal_is_not_retried() {
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(1_000, 307)).forbid_after(0),
    );
    let dir = tempfile::tempdir().unwrap();

    for path in ["/f.bin", "/missing"] {
        let err = engine()
            .download(&server.url(path), dir.path(), &mut |_| {})
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Status(_)), "{err}");
    }
    assert_eq!(server.gets("/f.bin").len(), 1);
    assert_eq!(server.gets("/missing").len(), 1);
}

#[tokio::test]
async fn resuming_tries_again_too() {
    // A download stopped by a cut (one try), then resumed while the server
    // has trouble: 503 twice, then the rest.
    let server = TestServer::http();
    let data = random_bytes(2_000_000, 308);
    let spec = FileSpec::new(data.clone());
    server.put("/f.bin", spec.clone().cut_after(700_000, 1));
    let dir = tempfile::tempdir().unwrap();
    let once = Engine::new(Options {
        connections: 1,
        max_failures: 1,
        ..Options::default()
    })
    .unwrap();
    let err = once
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();
    let Error::Interrupted { state_file, .. } = err else {
        panic!("{err}");
    };

    server.put(
        "/f.bin",
        spec.fail_first(vec![failure(503, None), failure(503, None)]),
    );
    let mut events = Vec::new();
    let done = engine()
        .resume(&state_file, &mut |e| events.push(e))
        .await
        .unwrap();

    assert_eq!(fs::read(&done.file).unwrap(), data);
    assert_eq!(retries(&events), 2);
    assert!(restarts(&events).is_empty(), "{events:?}");
    let gets = server.gets("/f.bin");
    assert!(gets.iter().skip(1).all(|r| r.range.is_some()), "{gets:?}");
}

/// AGENTS.md §10.5: cancelling while a download waits to try again, or
/// while it asks again, at many moments. A download that had data leaves
/// a state file that resumes to the right file; one that had none leaves
/// nothing. Either way the name is not lost.
#[tokio::test]
async fn cancelling_while_retrying_leaves_a_resumable_download_or_nothing() {
    let server = TestServer::http();
    let data = random_bytes(600_000, 309);
    let dir = tempfile::tempdir().unwrap();
    let state_file = dir.path().join("f.bin.zajil-state");
    for (cut, ms) in [0u64, 200_000]
        .into_iter()
        .flat_map(|cut| [0u64, 5, 20, 60, 150, 300, 500, 800, 1200].map(|ms| (cut, ms)))
    {
        // Breaks at the same byte on every try, and has no ranges.
        server.put(
            "/f.bin",
            FileSpec::new(data.clone())
                .without_ranges()
                .cut_after(cut, 1_000),
        );
        let cancelled = tokio::time::timeout(
            Duration::from_millis(ms),
            engine().download(&server.url("/f.bin"), dir.path(), &mut |_| {}),
        )
        .await;
        assert!(cancelled.is_err(), "{cut} bytes, {ms} ms: not cancelled");

        // The server works again: what was left must lead to the file.
        // A new download with no data yet may still be removing its files
        // (its first save was running): then the state file can vanish
        // before the resume reads it, and a new download is right.
        server.put("/f.bin", FileSpec::new(data.clone()));
        let deadline = Instant::now() + Duration::from_secs(10);
        let done = loop {
            let result = if state_file.exists() {
                engine().resume(&state_file, &mut |_| {}).await
            } else {
                engine()
                    .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
                    .await
            };
            match result {
                // The cancelled download's tasks may hold the part file a
                // moment longer (ARCHITECTURE §3.2).
                Err(Error::Locked(_)) | Err(Error::State(_)) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                other => break other.unwrap_or_else(|e| panic!("{cut}, {ms} ms: {e}")),
            }
        };
        assert_eq!(fs::read(&done.file).unwrap(), data, "{cut}, {ms} ms");
        // Nothing else is left, once the cancelled download is over.
        let deadline = Instant::now() + Duration::from_secs(10);
        while fs::read_dir(dir.path()).unwrap().count() > 1 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [done.file.file_name().unwrap()], "{cut}, {ms} ms");
        fs::remove_file(&done.file).unwrap();
    }
}

/// A server that is gone: port 1 on the loopback, where nothing listens
/// and no test server can be (they take free high ports; a port freed for
/// this could be handed to one: review of #51).
const GONE: &str = "http://127.0.0.1:1/f.bin";

/// #50: a server that is gone is tried a bounded number of times, with the
/// default settings a user has, then given up: never forever. Counted by
/// the retries the engine reports, not by time (review of #51: a refused
/// connection itself can take seconds on Windows).
#[tokio::test]
async fn a_server_that_is_gone_is_given_up_after_its_tries() {
    let dir = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let mut retries = 0;
    let result = Engine::new(Options::default())
        .unwrap()
        .download(GONE, dir.path(), &mut |event| {
            if matches!(event, Event::Retrying { .. }) {
                retries += 1;
            }
        })
        .await;
    assert!(result.is_err(), "{result:?}");
    assert_eq!(retries, zajil_core::DEFAULT_TRIES - 1, "{result:?}");
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "{:?}",
        started.elapsed()
    );
}
