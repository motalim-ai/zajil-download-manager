//! Dynamic splitting (M2): a free connection takes over half of the slowest
//! segment.

use std::fs;
use std::time::{Duration, Instant};

use zajil_core::state::DownloadState;
use zajil_core::{Engine, Event, Options};
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
