//! Downloads over several connections (M2) against the local test server.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use zajil_core::state::DownloadState;
use zajil_core::{Engine, Error, Event, Options, RestartReason};
use zajil_testserver::{FileSpec, RequestRecord, TestServer, random_bytes, sha256_hex};

const MIB: usize = 1024 * 1024;

fn engine(connections: usize) -> Engine {
    Engine::new(Options {
        connections,
        ..Options::default()
    })
    .unwrap()
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// The `(from, to)` of a `Range: bytes=from-to` request.
fn range_of(record: &RequestRecord) -> Option<(u64, u64)> {
    let spec = record.range.as_deref()?.strip_prefix("bytes=")?;
    let (from, to) = spec.split_once('-')?;
    Some((from.parse().ok()?, to.parse().ok()?))
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

#[tokio::test]
async fn downloads_50_mb_with_8_then_32_connections_and_the_sha256_matches() {
    let server = TestServer::http();
    let data = random_bytes(50 * MIB, 51);
    let expected = sha256_hex(&data);
    for connections in [8, 32] {
        let path = format!("/big-{connections}.bin");
        server.put(&path, FileSpec::new(data.clone()));
        let dir = tempfile::tempdir().unwrap();

        let done = engine(connections)
            .download(&server.url(&path), dir.path(), &mut |_| {})
            .await
            .unwrap();

        assert_eq!(done.bytes, 50 * MIB as u64);
        assert_eq!(sha256_hex(&fs::read(&done.file).unwrap()), expected);
        assert_eq!(names_in(dir.path()), [format!("big-{connections}.bin")]);
        // The first request has no range; then one range request for each
        // other segment of the plan: in order, without gaps or overlaps,
        // to the last byte. (Later changes may add requests, such as
        // retries, so the plan is checked, not the number of requests.)
        let gets = server.gets(&path);
        assert!(gets[0].range.is_none());
        let len = 50 * MIB as u64;
        let segment = len / connections as u64;
        let planned: Vec<(u64, u64)> = (1..connections as u64)
            .map(|i| {
                let end = if i + 1 == connections as u64 {
                    len - 1
                } else {
                    (i + 1) * segment - 1
                };
                (i * segment, end)
            })
            .collect();
        let asked: Vec<_> = gets[1..].iter().filter_map(range_of).collect();
        for range in &planned {
            assert!(
                asked.contains(range),
                "{range:?} was not asked for: {gets:?}"
            );
        }
        assert!(
            gets[1..]
                .iter()
                .all(|r| r.range.is_some() && r.status == 206)
        );
        assert!(asked.iter().all(|&(from, to)| from <= to && to < len));
    }
}

/// The benchmark of the M2 success criteria: a server that limits each
/// connection is faster with several. Also runs weekly in CI (`schedule`).
#[tokio::test]
async fn several_connections_beat_one_on_a_server_that_limits_each_connection() {
    let server = TestServer::http();
    let data = random_bytes(8 * MIB, 52);
    let per_connection = 2 * MIB as u64; // bytes per second
    let mut times = Vec::new();
    for connections in [1, 8] {
        let path = format!("/limited-{connections}.bin");
        server.put(&path, FileSpec::new(data.clone()).throttle(per_connection));
        let dir = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let done = engine(connections)
            .download(&server.url(&path), dir.path(), &mut |_| {})
            .await
            .unwrap();
        times.push(started.elapsed());
        assert_eq!(fs::read(&done.file).unwrap(), data);
        assert_eq!(server.most_concurrent(&path), connections);
    }
    let (one, eight) = (times[0], times[1]);
    eprintln!("1 connection: {one:?}, 8 connections: {eight:?}");
    // One connection needs about 4 s; eight need about 0.5 s.
    assert!(one >= Duration::from_secs(3), "{one:?}");
    assert!(
        eight * 3 < one,
        "8 connections took {eight:?}, 1 took {one:?}"
    );
}

#[tokio::test]
async fn a_server_without_range_support_gets_one_connection() {
    let server = TestServer::http();
    let data = random_bytes(10 * MIB, 53);
    server.put("/f.bin", FileSpec::new(data.clone()).without_ranges());
    let dir = tempfile::tempdir().unwrap();

    let done = engine(8)
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap();

    assert_eq!(fs::read(&done.file).unwrap(), data);
    let gets = server.gets("/f.bin");
    assert_eq!(gets.len(), 1);
    assert!(gets[0].range.is_none());
    let state = DownloadState::load(&dir.path().join("f.bin.zajil-state"));
    assert!(state.is_err(), "no state file is left");
}

#[tokio::test]
async fn a_server_that_announces_ranges_but_ignores_them_is_read_on_one_connection() {
    let server = TestServer::http();
    let data = random_bytes(10 * MIB, 54);
    server.put("/f.bin", FileSpec::new(data.clone()).ignore_ranges());
    let dir = tempfile::tempdir().unwrap();
    let mut events = Vec::new();

    let done = engine(8)
        .download(&server.url("/f.bin"), dir.path(), &mut |e| events.push(e))
        .await
        .unwrap();

    // The first connection read on through every segment; the range
    // requests got the whole file and were dropped, never written.
    assert_eq!(fs::read(&done.file).unwrap(), data);
    assert!(restarts(&events).is_empty());
    let gets = server.gets("/f.bin");
    assert!(gets.iter().all(|r| r.status == 200));
    assert_eq!(names_in(dir.path()), ["f.bin"]);
}

#[tokio::test]
async fn a_huge_announced_size_is_refused_before_any_file_is_created() {
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(1000, 55)).announce_length(1 << 60),
    );
    let dir = tempfile::tempdir().unwrap();

    let err = engine(8)
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();

    match err {
        Error::NotEnoughSpace {
            needed, available, ..
        } => {
            assert_eq!(needed, 1 << 60);
            assert!(available < needed);
        }
        other => panic!("expected NotEnoughSpace, got {other:?}"),
    }
    assert!(names_in(dir.path()).is_empty());
    assert_eq!(server.gets("/f.bin").len(), 1);
}

/// Every connection is cut after 300 kB; the download stops with a state of
/// several segments. Resuming asks for exactly the missing bytes of each.
async fn cut_parallel_download(server: &TestServer, dir: &Path, data: Vec<u8>) -> PathBuf {
    server.put("/f.bin", FileSpec::new(data).cut_after(300_000, 8));
    match engine(8)
        .download(&server.url("/f.bin"), dir, &mut |_| {})
        .await
    {
        Err(Error::Interrupted { state_file, .. }) => state_file,
        other => panic!("expected an interruption, got {other:?}"),
    }
}

#[tokio::test]
async fn a_cut_parallel_download_resumes_without_asking_twice() {
    let server = TestServer::http();
    let data = random_bytes(10 * MIB, 56);
    let dir = tempfile::tempdir().unwrap();
    let state_file = cut_parallel_download(&server, dir.path(), data.clone()).await;

    let state = DownloadState::load(&state_file).unwrap();
    assert_eq!(state.segments.len(), 8);
    assert!(state.bytes_done() > 0);
    let before = server.gets("/f.bin").len();

    let mut events = Vec::new();
    let done = engine(8)
        .resume(&state_file, &mut |e| events.push(e))
        .await
        .unwrap();
    assert_eq!(fs::read(&done.file).unwrap(), data);
    assert!(restarts(&events).is_empty());
    assert_eq!(names_in(dir.path()), ["f.bin"]);

    // Each segment was asked for from its first missing byte to its end.
    let mut asked: Vec<_> = server.gets("/f.bin")[before..]
        .iter()
        .map(|r| range_of(r).expect("resume asks for ranges"))
        .collect();
    asked.sort();
    let mut missing: Vec<_> = state
        .segments
        .iter()
        .filter(|s| s.start + s.done <= s.end.unwrap())
        .map(|s| (s.start + s.done, s.end.unwrap()))
        .collect();
    missing.sort();
    // Each missing range was asked for exactly (and any other request, such
    // as a retry in a later change, only for bytes inside a missing range).
    for range in &missing {
        assert!(asked.contains(range), "{range:?} not asked: {asked:?}");
    }
    for &(from, to) in &asked {
        assert!(
            missing.iter().any(|&(a, b)| a <= from && to <= b),
            "bytes {from}-{to} were already on disk"
        );
    }
    if asked.len() == missing.len() {
        let resent: u64 = server.gets("/f.bin")[before..]
            .iter()
            .map(|r| r.bytes_sent)
            .sum();
        assert_eq!(resent, data.len() as u64 - state.bytes_done());
    }
}

#[tokio::test]
async fn resuming_more_segments_than_connections_reuses_connections() {
    let server = TestServer::http();
    let data = random_bytes(10 * MIB, 57);
    let dir = tempfile::tempdir().unwrap();
    let state_file = cut_parallel_download(&server, dir.path(), data.clone()).await;
    let before = server.gets("/f.bin").len();

    let done = engine(2).resume(&state_file, &mut |_| {}).await.unwrap();
    assert_eq!(fs::read(&done.file).unwrap(), data);

    // Eight ranges over two connections: keep-alive reuses them.
    let resumed = &server.gets("/f.bin")[before..];
    assert!(resumed.len() >= 8);
    let mut connections: Vec<_> = resumed.iter().map(|r| r.connection).collect();
    connections.sort();
    connections.dedup();
    // Some connection carried several requests.
    assert!(connections.len() < resumed.len(), "{connections:?}");
    assert!(server.most_concurrent("/f.bin") <= 8);
}

#[tokio::test]
async fn a_file_that_changes_during_a_parallel_download_starts_over_from_the_new_version() {
    let server = TestServer::http();
    let old = random_bytes(10 * MIB, 58);
    let new = random_bytes(10 * MIB, 59);
    // The first request sees the old version; every range request after it
    // sees the new one, whose ETag no longer matches If-Range.
    server.put(
        "/f.bin",
        FileSpec::new(old).replaced_after(1, FileSpec::new(new.clone())),
    );
    let dir = tempfile::tempdir().unwrap();
    let mut events = Vec::new();

    let done = engine(8)
        .download(&server.url("/f.bin"), dir.path(), &mut |e| events.push(e))
        .await
        .unwrap();

    assert_eq!(restarts(&events), [RestartReason::FileChanged]);
    assert_eq!(sha256_hex(&fs::read(&done.file).unwrap()), sha256_hex(&new));
    assert_eq!(names_in(dir.path()), ["f.bin"]);
}

#[tokio::test]
async fn a_cancelled_parallel_download_without_data_leaves_nothing() {
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(10 * MIB, 60)).stall_after(0),
    );
    let dir = tempfile::tempdir().unwrap();

    let cancelled = tokio::time::timeout(
        Duration::from_millis(500),
        engine(8).download(&server.url("/f.bin"), dir.path(), &mut |_| {}),
    )
    .await;
    assert!(cancelled.is_err());
    // Writes and saves still on blocking threads end first.
    tokio::task::spawn_blocking(|| ()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        names_in(dir.path()).is_empty(),
        "{:?}",
        names_in(dir.path())
    );
}

#[tokio::test]
async fn a_linked_part_file_of_a_parallel_download_is_refused() {
    let server = TestServer::http();
    let data = random_bytes(10 * MIB, 61);
    let dir = tempfile::tempdir().unwrap();
    let state_file = cut_parallel_download(&server, dir.path(), data).await;

    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.txt");
    fs::write(&victim, b"precious").unwrap();
    let part = dir.path().join("f.bin.zajil-part");

    // A symbolic link and a hard link in place of the part file.
    fs::remove_file(&part).unwrap();
    zajil_platform::fs::symlink_file(&victim, &part).expect("symbolic links must work here");
    let err = engine(8)
        .resume(&state_file, &mut |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotRegularFile(_)), "{err:?}");
    fs::remove_file(&part).unwrap();

    fs::hard_link(&victim, &part).unwrap();
    let err = engine(8)
        .resume(&state_file, &mut |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotRegularFile(_)), "{err:?}");
    assert_eq!(fs::read(&victim).unwrap(), b"precious");
}

#[tokio::test]
async fn a_long_numbered_name_of_a_parallel_download_resumes() {
    let server = TestServer::http();
    let data = random_bytes(10 * MIB, 62);
    let dir = tempfile::tempdir().unwrap();
    // 200 bytes with two-byte Arabic letters, and the name is taken.
    let name = format!("{}.bin", "ز".repeat(98));
    assert_eq!(name.len(), 200);
    fs::write(dir.path().join(&name), b"old").unwrap();
    let disposition = format!(
        "attachment; filename*=UTF-8''{}",
        percent_encode(name.as_bytes())
    );
    server.put(
        "/f.bin",
        FileSpec::new(data.clone())
            .content_disposition(&disposition)
            .cut_after(300_000, 8),
    );
    let state_file = match engine(8)
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
    {
        Err(Error::Interrupted { state_file, .. }) => state_file,
        other => panic!("expected an interruption, got {other:?}"),
    };

    let done = engine(8).resume(&state_file, &mut |_| {}).await.unwrap();
    let file_name = done.file.file_name().unwrap().to_str().unwrap().to_owned();
    assert!(file_name.ends_with(" (1).bin"), "{file_name}");
    assert!(file_name.len() <= 200);
    assert_eq!(fs::read(&done.file).unwrap(), data);
    assert_eq!(fs::read(dir.path().join(&name)).unwrap(), b"old");
}

fn percent_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("%{b:02X}")).collect()
}

#[tokio::test]
async fn credentials_in_the_url_reach_every_range_request() {
    let server = TestServer::http();
    let data = random_bytes(8 * MIB, 63);
    // "Basic " and the base64 of "u:p".
    server.put(
        "/f.bin",
        FileSpec::new(data.clone()).require_authorization("Basic dTpw"),
    );
    let dir = tempfile::tempdir().unwrap();
    let url = server.url("/f.bin").replacen("http://", "http://u:p@", 1);

    let done = engine(8)
        .download(&url, dir.path(), &mut |_| {})
        .await
        .unwrap();

    assert_eq!(fs::read(&done.file).unwrap(), data);
    let gets = server.gets("/f.bin");
    assert_eq!(
        gets.iter().filter(|r| r.range.is_some()).count(),
        7,
        "{gets:?}"
    );
    assert!(gets.iter().all(|r| r.status != 401), "{gets:?}");
    assert!(
        gets.iter()
            .all(|r| r.authorization.as_deref() == Some("Basic dTpw"))
    );
}

#[tokio::test]
async fn credentials_never_go_to_another_origin() {
    let first = TestServer::http();
    let second = TestServer::http();
    let data = random_bytes(8 * MIB, 64);
    second.put("/f.bin", FileSpec::new(data.clone()));
    first.redirect("/f.bin", &second.url("/f.bin"), 302);
    let dir = tempfile::tempdir().unwrap();
    let url = first.url("/f.bin").replacen("http://", "http://u:p@", 1);

    let done = engine(8)
        .download(&url, dir.path(), &mut |_| {})
        .await
        .unwrap();

    assert_eq!(fs::read(&done.file).unwrap(), data);
    let gets = second.gets("/f.bin");
    assert!(gets.iter().filter(|r| r.range.is_some()).count() >= 7);
    assert!(gets.iter().all(|r| r.authorization.is_none()), "{gets:?}");
}

/// Review of #24: two machines behind one name, each with its own ETag for
/// the same file. Splitting sees "a changed file" again and again; the
/// download now starts over once, on one connection, and completes.
#[tokio::test]
async fn a_server_whose_etag_differs_per_connection_completes_on_one_connection() {
    let server = TestServer::http();
    let data = random_bytes(8 * MIB, 65);
    server.put(
        "/f.bin",
        FileSpec::new(data.clone()).etag_per_connection("\"backend-a\"", "\"backend-b\""),
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
    assert_eq!(restarts(&events), [RestartReason::FileChanged]);
    // After the restart nothing more was asked: the whole-file answer that
    // showed the change was read to the end on its own connection. So the
    // requests are the first one and the seven range requests only.
    let gets = server.gets("/f.bin");
    assert!(gets.len() <= 8, "{gets:?}");
    assert!(gets.iter().any(|r| r.range.is_some() && r.status == 200));
    assert_eq!(names_in(dir.path()), ["f.bin"]);
}
