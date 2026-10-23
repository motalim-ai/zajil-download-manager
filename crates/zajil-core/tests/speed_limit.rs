//! Speed limits (M3), measured on a clock the test owns (owner's decision
//! on the M3 plan): a `ManualClock` only moves when a limit makes a
//! download wait, so the time it shows is exactly what the limits cost,
//! however fast or slow CI is. The data still comes over real connections
//! from the test server. The one test on the real clock is in
//! `zajil-cli/tests/queue.rs` (`the_real_clock_drives_the_limits`).
//!
//! Owner's tolerance (ROADMAP M3): the average is between 0.9 and 1.1 MiB/s
//! for a limit of 1 MiB/s, over at least 6 seconds.

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use zajil_core::limit::{Clock, ManualClock, SpeedLimit};
use zajil_core::queue::{QueueStore, Runner};
use zajil_core::{Engine, Options, Stop};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

const MIB: u64 = 1024 * 1024;

fn engine(clock: &Arc<ManualClock>, connections: usize) -> Engine {
    Engine::new(Options {
        connections,
        clock: clock.clone(),
        ..Options::default()
    })
    .unwrap()
}

fn assert_rate(bytes: u64, took: Duration, rate: u64) {
    assert!(
        took >= Duration::from_secs(6),
        "measured over {took:?} only"
    );
    let speed = bytes as f64 / took.as_secs_f64() / rate as f64;
    assert!(
        (0.9..=1.1).contains(&speed),
        "{speed:.3} × the limit over {took:?}"
    );
}

/// M3 success criterion: a limit of 1 MiB/s for the whole program. Two
/// downloads at once (one on four connections, one on one) share it.
#[tokio::test]
async fn a_program_wide_limit_holds_for_two_downloads_together() {
    let server = TestServer::http();
    let (a, b) = (
        random_bytes(5 * MIB as usize, 700),
        random_bytes(3 * MIB as usize, 701),
    );
    server.put("/a.bin", FileSpec::new(a.clone()));
    server.put("/b.bin", FileSpec::new(b.clone()));
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new();
    let limit = SpeedLimit::new(Some(MIB), clock.clone());
    let start = clock.now();

    let mut tasks = Vec::new();
    for (path, connections) in [("/a.bin", 4), ("/b.bin", 1)] {
        let engine = engine(&clock, connections).with_speed_limit(Some(limit.clone()));
        let (url, dir) = (server.url(path), dir.path().to_path_buf());
        tasks.push(tokio::spawn(async move {
            engine.download(&url, &dir, &mut |_| {}).await.unwrap()
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }

    assert_rate(8 * MIB, clock.now() - start, MIB);
    assert_eq!(fs::read(dir.path().join("a.bin")).unwrap(), a);
    assert_eq!(fs::read(dir.path().join("b.bin")).unwrap(), b);
}

/// M3 success criterion, for each download on its own: two downloads at
/// once, each limited to 1 MiB/s, each get 1 MiB/s (together 2). Each one
/// has a clock of its own: a `ManualClock` jumps to the end of each wait,
/// so two limits that do not share a bucket must not share a clock either.
#[tokio::test]
async fn a_limit_for_each_download_holds_for_each_of_them() {
    let server = TestServer::http();
    let data = random_bytes(6 * MIB as usize, 702);
    server.put("/f.bin", FileSpec::new(data.clone()));
    let dir = tempfile::tempdir().unwrap();

    let mut tasks = Vec::new();
    for connections in [4, 1] {
        let clock = ManualClock::new();
        let engine = engine(&clock, connections).with_download_limit(Some(MIB));
        let (url, dir) = (server.url("/f.bin"), dir.path().to_path_buf());
        tasks.push(tokio::spawn(async move {
            let done = engine.download(&url, &dir, &mut |_| {}).await.unwrap();
            (done, clock.now())
        }));
    }
    for task in tasks {
        let (done, took) = task.await.unwrap();
        assert_rate(6 * MIB, took, MIB);
        assert_eq!(
            sha256_hex(&fs::read(&done.file).unwrap()),
            sha256_hex(&data)
        );
    }
}

/// No limit: the clock never moves.
#[tokio::test]
async fn without_a_limit_nothing_waits() {
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(random_bytes(3 * MIB as usize, 703)));
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new();
    engine(&clock, 4)
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(clock.now(), Duration::ZERO);
}

/// The queues' settings reach the downloads: the program-wide limit from
/// `settings.speed_limit`, and an item's own `limit`.
#[tokio::test]
async fn the_runner_applies_the_limits_of_the_queues_file() {
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(random_bytes(4 * MIB as usize, 704)));
    let data_dir = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    store
        .update(|q| {
            q.settings.speed_limit = Some(MIB);
            q.queue_or_new("main")?.max_active = 2;
            q.add("main", &server.url("/f.bin"), downloads.path())?;
            q.add("main", &server.url("/f.bin"), downloads.path())?;
            Ok(())
        })
        .unwrap();
    let clock = ManualClock::new();
    let runner = Runner::new(engine(&clock, 4), store.clone()).poll(Duration::from_millis(50));
    let summary = runner.run(&Stop::new(), &mut |_| {}).await.unwrap();
    assert_eq!(summary.finished, 2);
    assert_rate(8 * MIB, clock.now(), MIB);

    // An item's own limit, lower than the program's.
    let start = clock.now();
    store
        .update(|q| {
            let id = q.add("main", &server.url("/f.bin"), downloads.path())?;
            q.item_mut(id).unwrap().limit = Some(MIB / 2);
            Ok(())
        })
        .unwrap();
    let summary = runner.run(&Stop::new(), &mut |_| {}).await.unwrap();
    assert_eq!(summary.finished, 1);
    assert_rate(4 * MIB, clock.now() - start, MIB / 2);
}
