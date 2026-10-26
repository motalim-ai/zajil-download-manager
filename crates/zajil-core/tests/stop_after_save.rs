//! #132 (owner's decision): once a periodic save has succeeded, a stop does
//! not save again, so a slow disk cannot hold the stop; the download then
//! resumes from the last save (a little is downloaded again) to the right
//! file. Its own test binary: the save hook is global.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zajil_core::state::{DownloadState, test_hooks};
use zajil_core::{Engine, Error, Event, Options, Stop};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

/// The hook is global: one scenario at a time.
static ONE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const KIB: u64 = 1024;

fn engine(connections: usize, save_interval: Duration, min_segment: u64) -> Engine {
    Engine::new(Options {
        connections,
        save_interval,
        min_segment_size: min_segment,
        ..Options::default()
    })
    .unwrap()
}

/// Counts every state save; asks `stop` during save number `at` (if any),
/// so the download sees the stop right after that save has finished.
fn count_saves(stop: Stop, at: usize) -> Arc<AtomicUsize> {
    let saves = Arc::new(AtomicUsize::new(0));
    let seen = saves.clone();
    test_hooks::set_after_temporary_written(Some(Box::new(move |_| {
        if seen.fetch_add(1, Ordering::SeqCst) + 1 == at {
            stop.request();
        }
    })));
    saves
}

/// Resumes `state_file` and checks the file: the resume starts exactly at
/// the bytes the state file counts, and the result is `data`. (Not
/// compared with the last progress the test saw: a save counts every byte
/// synced, also those whose progress event was not handled yet; CI run of
/// #176: 180000 on disk, 100000 seen.)
async fn resumes_from_the_last_save(engine: &Engine, state_file: &std::path::Path, data: &[u8]) {
    let on_disk = DownloadState::load(state_file).unwrap().bytes_done();
    let from = Arc::new(Mutex::new(None));
    let seen = from.clone();
    let done = engine
        .resume(state_file, &mut |e| {
            if let Event::Started { from, .. } = e {
                seen.lock().unwrap().get_or_insert(from);
            }
        })
        .await
        .unwrap();
    assert_eq!(
        *from.lock().unwrap(),
        Some(on_disk),
        "the resume did not start at the last save"
    );
    assert_eq!(
        sha256_hex(&std::fs::read(&done.file).unwrap()),
        sha256_hex(data)
    );
    assert!(!state_file.exists());
}

async fn stops_after_a_save_without_saving_again(connections: usize, seed: u64) {
    let _one = ONE.lock().await;
    let server = TestServer::http();
    let data = random_bytes(3_000_000, seed);
    server.put("/f.bin", FileSpec::new(data.clone()).throttle(500_000));
    let dir = tempfile::tempdir().unwrap();
    let stop = Stop::new();
    // Save 1 is the new download's own; 2 and 3 are periodic.
    let saves = count_saves(stop.clone(), 3);
    let engine = engine(connections, Duration::from_millis(150), 1024 * KIB);

    let err = engine
        .download_until(&server.url("/f.bin"), dir.path(), &stop, &mut |_| {})
        .await
        .unwrap_err();
    test_hooks::set_after_temporary_written(None);
    let Error::Stopped {
        state_file: Some(state_file),
    } = err
    else {
        panic!("{err:?}");
    };
    // No save after the stop.
    assert_eq!(saves.load(Ordering::SeqCst), 3);
    resumes_from_the_last_save(&engine, &state_file, &data).await;
}

/// #194: the stop is asked during a save that takes longer than the save
/// interval (a slow disk, as on Windows CI). The next save is then due at
/// once, but none may come after the stop. Several rounds, as the save that
/// slipped through came only when it won a race.
async fn a_stop_during_a_slow_save_saves_no_more(connections: usize, seed: u64) {
    let _one = ONE.lock().await;
    for round in 0..5 {
        let server = TestServer::http();
        let data = random_bytes(1_500_000, seed + round);
        server.put("/f.bin", FileSpec::new(data.clone()).throttle(500_000));
        let dir = tempfile::tempdir().unwrap();
        let stop = Stop::new();
        // Save 2, the first periodic one, asks the stop, then takes 400 ms:
        // four save intervals.
        let saves = Arc::new(AtomicUsize::new(0));
        let (seen, asked) = (saves.clone(), stop.clone());
        test_hooks::set_after_temporary_written(Some(Box::new(move |_| {
            if seen.fetch_add(1, Ordering::SeqCst) + 1 == 2 {
                asked.request();
                std::thread::sleep(Duration::from_millis(400));
            }
        })));
        let engine = engine(connections, Duration::from_millis(100), 1024 * KIB);
        let err = engine
            .download_until(&server.url("/f.bin"), dir.path(), &stop, &mut |_| {})
            .await
            .unwrap_err();
        test_hooks::set_after_temporary_written(None);
        let Error::Stopped {
            state_file: Some(state_file),
        } = err
        else {
            panic!("round {round}: {err:?}");
        };
        assert_eq!(
            saves.load(Ordering::SeqCst),
            2,
            "round {round}: a save after the stop"
        );
        resumes_from_the_last_save(&engine, &state_file, &data).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_split_download_stopped_during_a_slow_save_saves_no_more() {
    a_stop_during_a_slow_save_saves_no_more(4, 140).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_single_connection_stopped_during_a_slow_save_saves_no_more() {
    a_stop_during_a_slow_save_saves_no_more(1, 150).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_split_download_stopped_after_a_save_does_not_save_again_and_resumes_whole() {
    stops_after_a_save_without_saving_again(4, 132).await;
}

/// One connection (no split): the resume asks for the rest from the last
/// save's offset.
#[tokio::test(flavor = "multi_thread")]
async fn a_single_connection_download_stopped_after_a_save_does_not_save_again_and_resumes_whole() {
    stops_after_a_save_without_saving_again(1, 133).await;
}

/// A split that came after the last save is not on disk: the state file
/// keeps the coarser segments, which still count only synced bytes, and the
/// resume continues from them.
#[tokio::test(flavor = "multi_thread")]
async fn a_split_after_the_last_save_is_resumed_from_the_coarser_segments() {
    let _one = ONE.lock().await;
    let server = TestServer::http();
    let data = random_bytes(4 * 1024 * 1024, 135);
    // The connection from byte 0 is slow, so the others finish and split it.
    server.put(
        "/f.bin",
        FileSpec::new(data.clone())
            .throttle(1024 * 1024)
            .slow_at(0, 64 * KIB),
    );
    let dir = tempfile::tempdir().unwrap();
    let stop = Stop::new();
    let saves = count_saves(Stop::new(), usize::MAX);
    let engine = engine(4, Duration::from_millis(400), 64 * KIB);
    let splits = Arc::new(AtomicUsize::new(0));
    let at_stop = Arc::new(AtomicUsize::new(usize::MAX));
    let (asked, counted, split_count, marker) =
        (stop.clone(), saves.clone(), splits.clone(), at_stop.clone());

    let err = engine
        .download_until(&server.url("/f.bin"), dir.path(), &stop, &mut |e| {
            // The first split once a periodic save is on disk: stop now,
            // so the split is newer than the last save.
            if let Event::Split { .. } = e {
                split_count.fetch_add(1, Ordering::SeqCst);
                if counted.load(Ordering::SeqCst) >= 2 && !asked.is_requested() {
                    marker.store(counted.load(Ordering::SeqCst), Ordering::SeqCst);
                    asked.request();
                }
            }
        })
        .await
        .unwrap_err();
    test_hooks::set_after_temporary_written(None);
    let Error::Stopped {
        state_file: Some(state_file),
    } = err
    else {
        panic!("{err:?}");
    };
    assert!(splits.load(Ordering::SeqCst) >= 1, "no split happened");
    assert_eq!(
        saves.load(Ordering::SeqCst),
        at_stop.load(Ordering::SeqCst),
        "a save after the stop"
    );
    let on_disk = DownloadState::load(&state_file).unwrap();
    assert!(
        on_disk.segments.len() < 4 + splits.load(Ordering::SeqCst),
        "the newest split is on disk: {} segments",
        on_disk.segments.len()
    );
    resumes_from_the_last_save(&engine, &state_file, &data).await;
}

/// Before any periodic save, a stop still saves what is on disk, as before:
/// with one connection and with a split download.
#[tokio::test(flavor = "multi_thread")]
async fn a_stop_before_any_periodic_save_saves_once() {
    let _one = ONE.lock().await;
    for (connections, seed) in [(1, 134), (4, 136)] {
        let server = TestServer::http();
        let data = random_bytes(3_000_000, seed);
        server.put("/f.bin", FileSpec::new(data.clone()).throttle(400_000));
        let dir = tempfile::tempdir().unwrap();
        let stop = Stop::new();
        let saves = count_saves(Stop::new(), usize::MAX);
        let engine = engine(connections, Duration::from_secs(60), 1024 * KIB);
        let at_stop = Arc::new(AtomicUsize::new(usize::MAX));
        let (asked, counted, marker) = (stop.clone(), saves.clone(), at_stop.clone());

        let err = engine
            .download_until(&server.url("/f.bin"), dir.path(), &stop, &mut |e| {
                if let Event::Progress { done, .. } = e
                    && done > 0
                    && !asked.is_requested()
                {
                    marker.store(counted.load(Ordering::SeqCst), Ordering::SeqCst);
                    asked.request();
                }
            })
            .await
            .unwrap_err();
        test_hooks::set_after_temporary_written(None);
        let Error::Stopped {
            state_file: Some(state_file),
        } = err
        else {
            panic!("{connections}: {err:?}");
        };
        assert_eq!(
            saves.load(Ordering::SeqCst),
            at_stop.load(Ordering::SeqCst) + 1,
            "{connections}: the stop saved once"
        );
        resumes_from_the_last_save(&engine, &state_file, &data).await;
    }
}
