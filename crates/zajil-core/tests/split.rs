//! Dynamic splitting (M2): a free connection takes over half of the slowest
//! segment.

use std::fs;
use std::time::{Duration, Instant};

use zajil_core::state::DownloadState;
use zajil_core::{Engine, Event, Options, RestartReason};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

const MIB: u64 = 1024 * 1024;
const KIB: u64 = 1024;

fn engine() -> Engine {
    Engine::new(Options {
        connections: 4,
        min_segment_size: 64 * KIB,
        ..Options::default()
    })
    .unwrap()
}

fn splits(events: &[Event]) -> Vec<(u64, u64)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Split { segment, at } => Some((*segment, *at)),
            _ => None,
        })
        .collect()
}

/// M2 success criterion: one very slow connection. Without splitting, its
/// 2 MiB segment alone would take 32 s at 64 KiB/s. It is the first
/// connection (the whole-file response), so no other connection can simply
/// read on into its segment: the segment has to be split.
#[tokio::test]
async fn one_slow_connection_is_split_and_the_download_ends_in_time() {
    let server = TestServer::http();
    let data = random_bytes(8 * MIB as usize, 70);
    // Four segments of 2 MiB; the first one is very slow.
    server.put("/f.bin", FileSpec::new(data.clone()).slow_at(0, 64 * KIB));
    let dir = tempfile::tempdir().unwrap();
    let mut events = Vec::new();

    let started = Instant::now();
    let done = engine()
        .download(&server.url("/f.bin"), dir.path(), &mut |e| events.push(e))
        .await
        .unwrap();
    let took = started.elapsed();

    assert_eq!(
        sha256_hex(&fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );
    let splits = splits(&events);
    eprintln!("took {took:?}, splits: {splits:?}");
    // The event log shows the slow segment being split, first of all.
    assert!(!splits.is_empty());
    assert_eq!(splits[0].0, 0, "{splits:?}");
    assert!(splits.iter().all(|&(segment, at)| at > segment));
    assert!(took < Duration::from_secs(8), "took {took:?}");
}

#[tokio::test]
async fn nothing_is_split_when_every_connection_is_fast() {
    let server = TestServer::http();
    let data = random_bytes(8 * MIB as usize, 71);
    server.put("/f.bin", FileSpec::new(data.clone()));
    let dir = tempfile::tempdir().unwrap();
    let mut events = Vec::new();
    let done = engine()
        .download(&server.url("/f.bin"), dir.path(), &mut |e| events.push(e))
        .await
        .unwrap();
    assert_eq!(fs::read(&done.file).unwrap(), data);
    assert!(splits(&events).is_empty(), "{:?}", splits(&events));
}

/// The state saved after a split describes the new segments; a download
/// stopped then resumes from it.
#[tokio::test]
async fn a_download_stopped_after_a_split_resumes_from_its_state() {
    let server = TestServer::http();
    let data = random_bytes(8 * MIB as usize, 72);
    let spec = FileSpec::new(data.clone())
        .etag(Some("\"same\""))
        .slow_at(0, 32 * KIB)
        .throttle(MIB);
    server.put("/f.bin", spec);
    let dir = tempfile::tempdir().unwrap();
    let state_file = dir.path().join("f.bin.zajil-state");

    // Stop the download once a state with more segments than connections
    // is on disk.
    let (first, url, mut ignore) = (engine(), server.url("/f.bin"), |_| {});
    let download = first.download(&url, dir.path(), &mut ignore);
    let watch = async {
        loop {
            if let Ok(state) = DownloadState::load(&state_file)
                && state.segments.len() > 4
            {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let saved = tokio::select! {
        result = download => panic!("the download ended first: {result:?}"),
        saved = tokio::time::timeout(Duration::from_secs(20), watch) => saved.unwrap(),
    };
    assert!(saved.bytes_done() > 0);
    // Dropping the download cancels its connections; they let go of the
    // file (and its lock) once their tasks and writes are over.
    tokio::time::sleep(Duration::from_millis(300)).await;
    tokio::task::spawn_blocking(|| ()).await.unwrap();

    server.put("/f.bin", FileSpec::new(data.clone()).etag(Some("\"same\"")));
    let mut events = Vec::new();
    let done = engine()
        .resume(&state_file, &mut |e| events.push(e))
        .await
        .unwrap();
    assert_eq!(
        sha256_hex(&fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );
    assert!(
        !events.iter().any(|e| matches!(e, Event::Restarted { .. })),
        "{events:?}"
    );
}

/// Owner decision on #28: when the file changes while it is being split,
/// the download starts over on one connection, and the final file must be
/// the new version in full, without a single byte of the old one, even
/// though most of the old version was already on disk.
#[tokio::test]
async fn a_change_during_splitting_leaves_only_the_new_version() {
    let server = TestServer::http();
    let old = random_bytes(8 * MIB as usize, 73);
    let new = random_bytes(8 * MIB as usize, 74);
    let new_spec = FileSpec::new(new.clone());
    let new_etag = new_spec.current_etag().unwrap().to_owned();
    // The first connection is slow, so the others finish their segments
    // (old bytes on disk) and then split it. Those split requests come
    // after the first four, when the server already has the new version.
    server.put(
        "/f.bin",
        FileSpec::new(old.clone())
            .slow_at(0, 64 * KIB)
            .replaced_after(4, new_spec),
    );
    let dir = tempfile::tempdir().unwrap();
    let mut events = Vec::new();

    let done = engine()
        .download(&server.url("/f.bin"), dir.path(), &mut |e| events.push(e))
        .await
        .unwrap();

    // Most of the old version was on disk before the change was seen.
    let restart = events
        .iter()
        .position(|e| matches!(e, Event::Restarted { .. }))
        .expect("the download started over");
    assert!(matches!(
        events[restart],
        Event::Restarted {
            reason: RestartReason::FileChanged
        }
    ));
    let old_on_disk = events[..restart]
        .iter()
        .filter_map(|e| match e {
            Event::Progress { done, .. } => Some(*done),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    assert!(
        old_on_disk >= 6 * MIB,
        "only {old_on_disk} old bytes were written"
    );
    assert!(!splits(&events[..restart]).is_empty());
    // One restart only, and then one connection: no range request was
    // ever made for the new version (a split download of it would send its
    // ETag in If-Range).
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Restarted { .. }))
            .count(),
        1
    );
    let gets = server.gets("/f.bin");
    assert!(
        gets.iter()
            .all(|r| r.if_range.as_deref() != Some(new_etag.as_str())),
        "{gets:?}"
    );

    // The new version in full; not one byte comes from the old one.
    let saved = fs::read(&done.file).unwrap();
    assert_eq!(saved.len(), new.len());
    assert_eq!(sha256_hex(&saved), sha256_hex(&new));
    let from_old = (0..saved.len())
        .filter(|&i| saved[i] == old[i] && old[i] != new[i])
        .count();
    assert_eq!(from_old, 0);
}

/// A server that slows a request for much (YouTube): with
/// `request_size`, no range request asks for more, and each connection
/// asks again for the next part of its segment (owner's report,
/// 2026-10-29).
#[tokio::test]
async fn range_requests_ask_for_at_most_the_request_size() {
    let server = TestServer::http();
    let data = random_bytes(6 * MIB as usize, 71);
    server.put("/f.bin", FileSpec::new(data.clone()));
    let dir = tempfile::tempdir().unwrap();
    let done = engine()
        .with_request_size(Some(MIB))
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(
        sha256_hex(&fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );
    let ranges: Vec<(u64, u64)> = server
        .gets("/f.bin")
        .iter()
        .filter_map(|r| {
            let spec = r.range.as_deref()?.strip_prefix("bytes=")?;
            let (from, to) = spec.split_once('-')?;
            Some((from.parse().ok()?, to.parse().ok()?))
        })
        .collect();
    assert!(ranges.len() >= 5, "{ranges:?}");
    for (from, to) in &ranges {
        assert!(to - from < MIB, "{from}-{to}");
    }
}
