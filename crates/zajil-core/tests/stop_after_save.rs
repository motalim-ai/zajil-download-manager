//! #132 (owner's decision): once a periodic save has succeeded, a stop does
//! not save again, so a slow disk cannot hold the stop; the download still
//! resumes to the right file. Its own test binary: the save hook is global.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use zajil_core::state::test_hooks;
use zajil_core::{Engine, Error, Options, Stop};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

/// The hook is global: one scenario at a time.
static ONE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn engine(connections: usize, save_interval: Duration) -> Engine {
    Engine::new(Options {
        connections,
        save_interval,
        ..Options::default()
    })
    .unwrap()
}

/// Counts every state save; asks `stop` during save number `at`, so the
/// download sees the stop right after that save has finished.
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

async fn stops_after_a_save_without_saving_again(
    spec: FileSpec,
    data: Vec<u8>,
    connections: usize,
) {
    let server = TestServer::http();
    server.put("/f.bin", spec);
    let dir = tempfile::tempdir().unwrap();
    let stop = Stop::new();
    // Save 1 is the new download's own; 2 and 3 are periodic.
    let saves = count_saves(stop.clone(), 3);
    let engine = engine(connections, Duration::from_millis(150));

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

    // The file still comes out right.
    let done = engine.resume(&state_file, &mut |_| {}).await.unwrap();
    assert_eq!(
        sha256_hex(&std::fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );
    assert!(!state_file.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_split_download_stopped_after_a_save_does_not_save_again_and_resumes_whole() {
    let _one = ONE.lock().await;
    let data = random_bytes(3_000_000, 132);
    stops_after_a_save_without_saving_again(FileSpec::new(data.clone()).throttle(500_000), data, 4)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_single_connection_download_stopped_after_a_save_does_not_save_again_and_resumes_whole() {
    let _one = ONE.lock().await;
    let data = random_bytes(3_000_000, 133);
    stops_after_a_save_without_saving_again(
        FileSpec::new(data.clone())
            .without_ranges()
            .throttle(500_000),
        data,
        1,
    )
    .await;
}

/// Before any periodic save, a stop still saves what is on disk, as before.
#[tokio::test(flavor = "multi_thread")]
async fn a_stop_before_any_periodic_save_saves_once() {
    let _one = ONE.lock().await;
    let server = TestServer::http();
    let data = random_bytes(2_000_000, 134);
    server.put("/f.bin", FileSpec::new(data.clone()).throttle(400_000));
    let dir = tempfile::tempdir().unwrap();
    let stop = Stop::new();
    let saves = count_saves(Stop::new(), usize::MAX);
    let engine = engine(4, Duration::from_secs(60));
    let at_stop = Arc::new(AtomicUsize::new(usize::MAX));
    let (asked, counted) = (stop.clone(), saves.clone());
    let marker = at_stop.clone();

    let err = engine
        .download_until(&server.url("/f.bin"), dir.path(), &stop, &mut |e| {
            if matches!(e, zajil_core::Event::Progress { done, .. } if done > 0)
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
        panic!("{err:?}");
    };
    assert_eq!(
        saves.load(Ordering::SeqCst),
        at_stop.load(Ordering::SeqCst) + 1
    );
    let done = engine.resume(&state_file, &mut |_| {}).await.unwrap();
    assert_eq!(
        sha256_hex(&std::fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );
}
