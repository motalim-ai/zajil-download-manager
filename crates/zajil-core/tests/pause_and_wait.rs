//! "Stop and wait" (#5, deferred from the review of #15): a download
//! stopped with [`Stop`] returns only when nothing of it runs any more, so
//! no older state save can land after a resume in the same process.
//!
//! A slow save is made with the global save hook (`test-support`), so the
//! tests of this file run one at a time, in a binary of their own.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use zajil_core::state::{DownloadState, test_hooks};
use zajil_core::{Engine, Error, Event, Options, Stop};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

const MIB: usize = 1024 * 1024;

/// One test at a time: the save hook is global.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A save hook that, once armed, holds the next save of the state file
/// until the gate opens, and counts the saves that are running.
#[derive(Default)]
struct SlowSave {
    armed: AtomicBool,
    running: AtomicUsize,
    held: (Mutex<bool>, Condvar),
    open: (Mutex<bool>, Condvar),
}

impl SlowSave {
    fn install(self: &Arc<Self>) {
        let me = self.clone();
        test_hooks::set_after_temporary_written(Some(Box::new(move |tmp: &Path| {
            if !tmp.to_string_lossy().ends_with(".zajil-state.tmp")
                || !me.armed.swap(false, Ordering::SeqCst)
            {
                return;
            }
            me.running.fetch_add(1, Ordering::SeqCst);
            *me.held.0.lock().unwrap() = true;
            me.held.1.notify_all();
            let mut open = me.open.0.lock().unwrap();
            while !*open {
                open = me.open.1.wait(open).unwrap();
            }
            me.running.fetch_sub(1, Ordering::SeqCst);
        })));
    }

    /// Blocks (on a blocking thread) until a save is held.
    async fn wait_until_held(self: &Arc<Self>) {
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut held = me.held.0.lock().unwrap();
            while !*held {
                held = me.held.1.wait(held).unwrap();
            }
        })
        .await
        .unwrap();
    }

    /// Lets the held save finish after `delay`: a save that is slow (an
    /// antivirus, a busy disk).
    fn open_after(self: &Arc<Self>, delay: Duration) {
        let me = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            *me.open.0.lock().unwrap() = true;
            me.open.1.notify_all();
        });
    }
}

struct Uninstall;
impl Drop for Uninstall {
    fn drop(&mut self) {
        test_hooks::set_after_temporary_written(None);
    }
}

fn engine(connections: usize) -> Engine {
    Engine::new(Options {
        connections,
        save_interval: Duration::from_millis(50),
        ..Options::default()
    })
    .unwrap()
}

/// Starts a download whose next state save, once some data is on disk, is
/// held; returns its task, the stop signal and the hook.
async fn download_held_in_a_save(
    server: &TestServer,
    dir: &Path,
    connections: usize,
) -> (
    tokio::task::JoinHandle<Result<zajil_core::Finished, Error>>,
    Stop,
    Arc<SlowSave>,
) {
    let slow = Arc::new(SlowSave::default());
    slow.install();
    let stop = Stop::new();
    let (url, dir, task_stop, arm) = (
        server.url("/f.bin"),
        dir.to_path_buf(),
        stop.clone(),
        slow.clone(),
    );
    let task = tokio::spawn(async move {
        engine(connections)
            .download_until(&url, &dir, &task_stop, &mut |event| {
                if let Event::Progress { done, .. } = event
                    && done > MIB as u64 / 2
                {
                    arm.armed.store(true, Ordering::SeqCst);
                }
            })
            .await
    });
    slow.wait_until_held().await;
    (task, stop, slow)
}

fn assert_unlocked(part: &Path) {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(part)
        .unwrap();
    file.try_lock().expect("the part file is no longer locked");
}

async fn stop_during_a_slow_save_then_resume_at_once(connections: usize) {
    let _one = ONE_AT_A_TIME.lock().await;
    let _uninstall = Uninstall;
    let server = TestServer::http();
    let data = random_bytes(8 * MIB, 500 + connections as u64);
    server.put(
        "/f.bin",
        FileSpec::new(data.clone()).throttle(2 * MIB as u64),
    );
    let dir = tempfile::tempdir().unwrap();

    let (task, stop, slow) = download_held_in_a_save(&server, dir.path(), connections).await;
    // Stop while the save is held; it finishes 300 ms later.
    stop.request();
    slow.open_after(Duration::from_millis(300));
    let result = task.await.unwrap();

    // When the call returns, no save of it is running any more, and the
    // part file is closed and unlocked.
    assert_eq!(
        slow.running.load(Ordering::SeqCst),
        0,
        "a save outlived the stop"
    );
    let state_file = match result {
        Err(Error::Stopped {
            state_file: Some(state_file),
        }) => state_file,
        other => panic!("expected a stop, got {other:?}"),
    };
    let part = dir.path().join("f.bin.zajil-part");
    assert_unlocked(&part);
    let stopped_at = DownloadState::load(&state_file).unwrap().bytes_done();
    assert!(stopped_at > 0);

    // Resume at once, in the same process: it starts from the newest state
    // and finishes with the right file.
    let mut from = None;
    let done = engine(connections)
        .resume_until(&state_file, &Stop::new(), &mut |event| {
            if let Event::Started { from: f, .. } = event {
                from.get_or_insert(f);
            }
        })
        .await
        .unwrap();
    assert_eq!(from, Some(stopped_at));
    assert_eq!(sha256_hex(&fs::read(done.file).unwrap()), sha256_hex(&data));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_single_connection_stopped_during_a_slow_save_resumes_from_the_newest_state() {
    stop_during_a_slow_save_then_resume_at_once(1).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_segmented_download_stopped_during_a_slow_save_resumes_from_the_newest_state() {
    stop_during_a_slow_save_then_resume_at_once(4).await;
}

/// The control: cancelling by dropping the future (what M1 and M2 did)
/// leaves the slow save running after the call is gone. This is what the
/// test above would catch.
#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_future_instead_leaves_the_save_running() {
    let _one = ONE_AT_A_TIME.lock().await;
    let _uninstall = Uninstall;
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(8 * MIB, 510)).throttle(2 * MIB as u64),
    );
    let dir = tempfile::tempdir().unwrap();

    let (task, _stop, slow) = download_held_in_a_save(&server, dir.path(), 1).await;
    task.abort();
    let _ = task.await;
    assert_eq!(slow.running.load(Ordering::SeqCst), 1);
    slow.open_after(Duration::ZERO);
}

/// A stop before any data leaves no files, and returns no state file.
#[tokio::test]
async fn a_new_download_stopped_before_any_data_leaves_nothing() {
    let _one = ONE_AT_A_TIME.lock().await;
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(MIB, 511)).stall_after(0),
    );
    let dir = tempfile::tempdir().unwrap();
    let stop = Stop::new();
    let mut started = false;
    let task_stop = stop.clone();
    let result = engine(1)
        .download_until(&server.url("/f.bin"), dir.path(), &stop, &mut |event| {
            if matches!(event, Event::Started { .. }) && !started {
                started = true;
                task_stop.request();
            }
        })
        .await;
    assert!(
        matches!(result, Err(Error::Stopped { state_file: None })),
        "{result:?}"
    );
    let left: Vec<PathBuf> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert!(left.is_empty(), "{left:?}");
}
