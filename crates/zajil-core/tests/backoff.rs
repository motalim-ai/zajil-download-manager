//! Retries and busy servers (M2): growing waits with a random part, `429`
//! with `Retry-After`, and `403` when the server limits connections.

use std::fs;
use std::time::Duration;

use zajil_core::state::DownloadState;
use zajil_core::{Engine, Error, Event, Options};
use zajil_testserver::{Failure, FileSpec, RequestRecord, TestServer, random_bytes, sha256_hex};

const MIB: usize = 1024 * 1024;

fn engine(connections: usize) -> Engine {
    Engine::new(Options {
        connections,
        ..Options::default()
    })
    .unwrap()
}

/// `(status, connections, wait)` of every `ServerBusy` event.
fn busy(events: &[Event]) -> Vec<(u16, usize, Option<Duration>)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ServerBusy {
                status,
                connections,
                wait,
            } => Some((*status, *connections, *wait)),
            _ => None,
        })
        .collect()
}

fn with_status(records: &[RequestRecord], status: u16) -> Vec<&RequestRecord> {
    records.iter().filter(|r| r.status == status).collect()
}

/// M2 success criterion: `429` with `Retry-After: 2`. No request is sent
/// before the two seconds are over, and the number of connections drops.
#[tokio::test]
async fn too_many_requests_waits_for_retry_after_and_uses_fewer_connections() {
    let server = TestServer::http();
    let data = random_bytes(8 * MIB, 80);
    // Two bodies at once at most; each connection is slow enough that the
    // first two are still sending when the others ask.
    server.put(
        "/f.bin",
        FileSpec::new(data.clone())
            .throttle(4 * MIB as u64)
            .too_many_beyond(2, 2),
    );
    let dir = tempfile::tempdir().unwrap();
    let mut events = Vec::new();

    let done = engine(8)
        .download(&server.url("/f.bin"), dir.path(), &mut |e| events.push(e))
        .await
        .unwrap();
    assert_eq!(
        sha256_hex(&fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );

    let gets = server.gets("/f.bin");
    let refused = with_status(&gets, 429);
    assert!(!refused.is_empty());
    // After each 429, nothing was asked for two seconds. Requests already
    // on their way when it was sent (the same burst) arrive within a few
    // milliseconds of it.
    for answer in &refused {
        for request in &gets {
            let after = request.at.saturating_sub(answer.at);
            assert!(
                request.at <= answer.at + Duration::from_millis(100)
                    || after >= Duration::from_millis(1900),
                "a request {after:?} after a 429: {gets:?}"
            );
        }
    }
    // The connections dropped to what the server accepts (two, or fewer if
    // the server still counted a connection the client had closed), never
    // back up, and the events said so, with the wait.
    let busy = busy(&events);
    assert!(
        busy.iter()
            .all(|&(status, _, wait)| status == 429 && wait == Some(Duration::from_secs(2)))
    );
    assert!(busy.windows(2).all(|w| w[1].1 <= w[0].1), "{busy:?}");
    assert!(busy.last().unwrap().1 <= 2, "{busy:?}");
    assert!(server.most_concurrent("/f.bin") <= 2);
}

/// M2 success criterion: `403` after the fourth connection. The download
/// completes with four connections or fewer, and the file is whole.
#[tokio::test]
async fn forbidden_beyond_four_connections_completes_with_four_or_fewer() {
    let server = TestServer::http();
    let data = random_bytes(16 * MIB, 81);
    server.put(
        "/f.bin",
        FileSpec::new(data.clone())
            .throttle(8 * MIB as u64)
            .max_concurrent(4),
    );
    let dir = tempfile::tempdir().unwrap();
    let mut events = Vec::new();

    let done = engine(8)
        .download(&server.url("/f.bin"), dir.path(), &mut |e| events.push(e))
        .await
        .unwrap();
    assert_eq!(
        sha256_hex(&fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );

    let gets = server.gets("/f.bin");
    let refused = with_status(&gets, 403);
    // Each 403 lowers the limit by one at least, so the server is never
    // hammered: at most 8 - 1 refusals. The limit reached four (or fewer,
    // if the server still counted a connection the client had closed) and
    // never went back up.
    assert!(!refused.is_empty() && refused.len() <= 7, "{gets:?}");
    let busy = busy(&events);
    assert!(
        busy.iter()
            .all(|&(status, _, wait)| status == 403 && wait.is_none())
    );
    assert!(busy.windows(2).all(|w| w[1].1 < w[0].1), "{busy:?}");
    assert!(busy.last().unwrap().1 <= 4, "{busy:?}");
    assert!(server.most_concurrent("/f.bin") <= 4);
}

/// Cut connections are tried again after a short wait, from the first
/// missing byte, and the download completes without stopping.
#[tokio::test]
async fn cut_connections_are_retried_from_where_they_stopped() {
    let server = TestServer::http();
    let data = random_bytes(10 * MIB, 82);
    server.put("/f.bin", FileSpec::new(data.clone()).cut_after(300_000, 8));
    let dir = tempfile::tempdir().unwrap();

    let done = engine(8)
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(
        sha256_hex(&fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );
    // Each retry continues after what its segment kept: it asks for the
    // same end from a later byte.
    let gets = server.gets("/f.bin");
    let range = |r: &RequestRecord| {
        let (from, to) = r
            .range
            .as_deref()?
            .strip_prefix("bytes=")?
            .split_once('-')?;
        Some((from.parse::<u64>().ok()?, to.parse::<u64>().ok()?))
    };
    let mut retries = 0;
    for (i, later) in gets.iter().enumerate() {
        let Some((from, to)) = range(later) else {
            continue;
        };
        if let Some((earlier_from, _)) = gets[..i]
            .iter()
            .filter_map(range)
            .rfind(|&(_, end)| end == to)
        {
            assert!(
                from > earlier_from,
                "bytes {from}-{to} asked again from the start"
            );
            retries += 1;
        }
    }
    assert!(retries >= 7, "{gets:?}");
    // How many bytes the server handed over is no measure of what arrived:
    // after a cut (a TCP reset), Windows drops data the client had not read
    // yet. What matters is that every retry continues after what was kept.
}

/// A segment whose connection keeps failing is given up after a few tries
/// with growing waits; the download stops resumable and resumes later.
#[tokio::test]
async fn a_segment_that_keeps_failing_is_given_up_after_growing_waits() {
    let server = TestServer::http();
    let data = random_bytes(4 * MIB, 83);
    // The first response is cut; every range request gets 503.
    server.put(
        "/f.bin",
        FileSpec::new(data.clone())
            .cut_after(MIB as u64 / 2, 1)
            .fail_range_requests(Failure {
                status: 503,
                retry_after: None,
            }),
    );
    let dir = tempfile::tempdir().unwrap();

    let err = engine(4)
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();
    let Error::Interrupted { state_file, reason } = err else {
        panic!("expected an interruption, got {err:?}");
    };
    assert!(
        reason.contains("503") && reason.contains("gave up after 5 tries"),
        "{reason}"
    );

    // The tries of one segment: five, each wait at least half its ceiling
    // (0.5 s, 1 s, 2 s, 4 s), growing.
    let gets = server.gets("/f.bin");
    let first_range = gets.iter().find_map(|r| r.range.clone()).unwrap();
    let tries: Vec<Duration> = gets
        .iter()
        .filter(|r| r.range.as_ref() == Some(&first_range))
        .map(|r| r.at)
        .collect();
    assert_eq!(tries.len(), 5, "{tries:?}");
    let waits: Vec<Duration> = tries.windows(2).map(|w| w[1] - w[0]).collect();
    for (i, wait) in waits.iter().enumerate() {
        let floor = Duration::from_millis(250 << i);
        assert!(*wait >= floor, "wait {i} was {wait:?}");
    }

    // Once the server is fine again, the download resumes.
    assert!(DownloadState::load(&state_file).unwrap().bytes_done() > 0);
    server.put("/f.bin", FileSpec::new(data.clone()));
    let done = engine(4).resume(&state_file, &mut |_| {}).await;
    // A new ETag means a new version: it starts over, but it completes.
    assert_eq!(fs::read(&done.unwrap().file).unwrap(), data);
}

/// A server asking for a wait longer than ten minutes is not waited for.
#[tokio::test]
async fn a_very_long_retry_after_stops_the_download_at_once() {
    let server = TestServer::http();
    let data = random_bytes(4 * MIB, 84);
    server.put(
        "/f.bin",
        FileSpec::new(data)
            .cut_after(MIB as u64 / 2, 1)
            .fail_range_requests(Failure {
                status: 429,
                retry_after: Some(3600),
            }),
    );
    let dir = tempfile::tempdir().unwrap();

    let started = std::time::Instant::now();
    let err = engine(4)
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5));
    let Error::Interrupted { reason, .. } = err else {
        panic!("expected an interruption, got {err:?}");
    };
    assert!(reason.contains("3600 s"), "{reason}");
}
