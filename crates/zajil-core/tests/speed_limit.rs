//! Speed limits (M3), measured on a clock the test owns (owner's decision
//! on the M3 plan): a `ManualClock` only moves when a limit makes a
//! download wait, so the time it shows is exactly what the limits cost,
//! however fast or slow CI is. The data still comes over real connections
//! from the test server. The tests on the real clock are
//! `the_real_clock_drives_limits_and_schedules` in `zajil-cli/tests/queue.rs`,
//! and `speed_limit_real_clock.rs`, which runs every night only (#36).
//!
//! Owner's tolerance (ROADMAP M3): the average is between 0.9 and 1.1 MiB/s
//! for a limit of 1 MiB/s, over at least 6 seconds.

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use zajil_core::limit::{Clock, ManualClock, SpeedLimit};
use zajil_core::queue::{QueueStore, Runner};
use zajil_core::{Engine, Options, Stop};
use zajil_testserver::{FileSpec, Gate, TestServer, random_bytes, sha256_hex};

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

/// Review of #32: on many connections, each wrote a whole batch (256 KiB)
/// before the limit counted anything, so the first second went 3 to 6
/// times faster than the limit (only the average was right). Now every
/// piece is counted as it arrives: at no moment is more on disk than the
/// rate allows by then, plus a burst and a piece or so per connection.
#[tokio::test]
async fn the_limit_holds_from_the_first_second_not_only_on_average() {
    use std::sync::Mutex;
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(random_bytes(8 * MIB as usize, 705)));
    let dir = tempfile::tempdir().unwrap();
    for connections in [8, 32] {
        let clock = ManualClock::new();
        let rate = MIB / 2;
        let engine = engine(&clock, connections).with_download_limit(Some(rate));
        let worst = Mutex::new((0i64, Duration::ZERO, 0u64));
        let at = clock.clone();
        engine
            .download(&server.url("/f.bin"), dir.path(), &mut |event| {
                if let zajil_core::Event::Progress { done, .. } = event {
                    let now = at.now();
                    let allowed = (rate as f64 * now.as_secs_f64()) as i64;
                    let ahead = done as i64 - allowed;
                    let mut worst = worst.lock().unwrap();
                    if ahead > worst.0 {
                        *worst = (ahead, now, done);
                    }
                }
            })
            .await
            .unwrap();
        let (ahead, when, done) = *worst.lock().unwrap();
        // A tenth of a second of burst, and 512 KiB for pieces in flight.
        let slack = rate / 10 + 512 * 1024;
        assert!(
            ahead <= slack as i64,
            "{connections} connections: {done} bytes on disk at {when:?}, \
             {ahead} ahead of the limit"
        );
        for entry in fs::read_dir(dir.path()).unwrap() {
            fs::remove_file(entry.unwrap().path()).unwrap();
        }
    }
}

/// #106: a download's own limit is a handle its caller keeps: a new rate
/// reaches the running download at once, up or down, and removing it lets
/// the download go without waiting any more.
#[tokio::test]
async fn a_running_download_takes_a_new_limit_of_its_own() {
    let server = TestServer::http();
    let data = random_bytes(12 * MIB as usize, 706);
    server.put("/f.bin", FileSpec::new(data.clone()));
    let dir = tempfile::tempdir().unwrap();

    // Up, then down: one rate for eight seconds, then the other. Each rate
    // is measured between two moments of steady flow, away from the change
    // and the end: pieces taken but not yet written would count on the
    // wrong side of the change. One connection, so that one piece at most
    // is in flight (four make a MiB, four seconds at a quarter MiB/s); it
    // reports a piece before it waits for it, so what is paid at a mark is
    // what it had before that piece.
    for (first, second) in [(MIB / 4, MIB), (MIB, MIB / 4)] {
        let clock = ManualClock::new();
        let limit = SpeedLimit::new(Some(first), clock.clone());
        let limited = engine(&clock, 1).with_own_limit(limit.clone());
        let start = MIB / 2;
        let change = start + 8 * first;
        let marks = [start, change, change + MIB, change + MIB + 8 * second];
        let mut seen: Vec<(u64, Duration)> = Vec::new();
        let mut paid = 0;
        let at = clock.clone();
        let finished = limited
            .download(&server.url("/f.bin"), dir.path(), &mut |event| {
                let zajil_core::Event::Progress { done, .. } = event else {
                    return;
                };
                let before = std::mem::replace(&mut paid, done);
                if marks.get(seen.len()).is_some_and(|&mark| done >= mark) {
                    seen.push((before, at.now()));
                    if seen.len() == 2 {
                        limit.set_rate(Some(second));
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(
            sha256_hex(&fs::read(&finished.file).unwrap()),
            sha256_hex(&data)
        );
        assert_eq!(seen.len(), 4, "{seen:?}");
        assert_rate(seen[1].0 - seen[0].0, seen[1].1 - seen[0].1, first);
        assert_rate(seen[3].0 - seen[2].0, seen[3].1 - seen[2].1, second);
        fs::remove_file(&finished.file).unwrap();
    }

    // No limit, then one: the case of the «Speed limit» tab on a download
    // that runs without one. On one connection and on four (the split
    // download takes its limits elsewhere). The server holds every
    // response at three places, each until the test opens its gate: the
    // change comes while the first one holds them, so it comes while the
    // data does; the rate is measured between the second and the third,
    // where every byte that came is written and paid for, and nothing is
    // in flight. Measured between moments of steady flow instead, four
    // connections had up to a piece each taken but not yet reported on
    // either side (#274: 1.103 × the limit, 3 pieces of 64 KiB over 2 MiB).
    let slow = random_bytes(6 * MIB as usize, 708);
    for connections in [1, 4] {
        // From the first byte of each response: four of 1.5 MiB, or one of
        // 6 MiB; 3 MiB between the second and third holds either way.
        let holds = if connections == 1 {
            [MIB / 4, MIB, 4 * MIB]
        } else {
            [MIB / 4, MIB / 2, 5 * MIB / 4]
        };
        let gates = [Gate::new(), Gate::new(), Gate::new()];
        let mut spec = FileSpec::new(slow.clone());
        for (at, gate) in holds.iter().zip(&gates) {
            spec = spec.hold_after(*at, gate);
        }
        server.put("/slow.bin", spec);
        let clock = ManualClock::new();
        let limit = SpeedLimit::new(None, clock.clone());
        let limited = engine(&clock, connections).with_own_limit(limit.clone());
        let all = connections as u64;
        let mut seen: Vec<(u64, Duration)> = Vec::new();
        let mut paid = 0;
        let at = clock.clone();
        let finished = limited
            .download(&server.url("/slow.bin"), dir.path(), &mut |event| {
                let zajil_core::Event::Progress { done, .. } = event else {
                    return;
                };
                // One connection reports a piece before it waits for it.
                let before = std::mem::replace(&mut paid, done);
                let counted = if connections == 1 { before } else { done };
                if !gates[0].is_open() {
                    assert_eq!(at.now(), Duration::ZERO, "it waited without a limit");
                    limit.set_rate(Some(MIB / 4));
                    gates[0].open();
                } else if !gates[1].is_open() && done == all * holds[1] {
                    seen.push((counted, at.now()));
                    gates[1].open();
                } else if !gates[2].is_open() && done == all * holds[2] {
                    seen.push((counted, at.now()));
                    gates[2].open();
                }
            })
            .await
            .unwrap();
        assert_eq!(seen.len(), 2, "{connections}: {seen:?}");
        assert_eq!(fs::read(&finished.file).unwrap(), slow);
        assert_rate(seen[1].0 - seen[0].0, seen[1].1 - seen[0].1, MIB / 4);
        fs::remove_file(&finished.file).unwrap();
    }

    // Limited, then no limit: nothing waits after the change. One
    // connection, whose events come as it goes: the change is made with
    // most of the file still to come.
    let clock = ManualClock::new();
    let limit = SpeedLimit::new(Some(MIB / 4), clock.clone());
    let limited = engine(&clock, 1).with_own_limit(limit.clone());
    let mut removed = None;
    let at = clock.clone();
    limited
        .download(&server.url("/f.bin"), dir.path(), &mut |event| {
            if let zajil_core::Event::Progress { done, .. } = event
                && done >= MIB
                && removed.is_none()
            {
                limit.set_rate(None);
                removed = Some((done, at.now()));
            }
        })
        .await
        .unwrap();
    let (done, removed_at) = removed.expect("the limit was never removed");
    assert!(done <= 2 * MIB, "removed only at {done} bytes");
    // It was limited until then (without a limit the clock never moves;
    // a piece of a connection can be half a MiB).
    assert!(removed_at >= Duration::from_secs(1), "{removed_at:?}");
    // The piece already waiting finishes its wait: a second at most.
    assert!(
        clock.now() - removed_at <= Duration::from_secs(1),
        "it still waited {:?} after the limit was removed",
        clock.now() - removed_at
    );
}

/// #106: the runner reads an item's limit at each look and gives it to the
/// download that runs: removed in the queues file (as the progress
/// window's «Speed limit» tab does), it stops limiting at once. On the
/// real clock: at the first limit the download would take 32 seconds.
#[tokio::test]
async fn the_runner_gives_a_running_download_the_limit_of_its_item() {
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(random_bytes(2 * MIB as usize, 707)));
    let data_dir = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let id = store
        .update(|q| {
            let id = q.add("main", &server.url("/f.bin"), downloads.path())?;
            q.item_mut(id).unwrap().limit = Some(64 * 1024);
            Ok(id)
        })
        .unwrap();
    let engine = Engine::new(Options::default()).unwrap();
    let runner = Runner::new(engine, store.clone()).poll(Duration::from_millis(50));
    let steering = store.clone();
    let mut lifted = false;
    let started = std::time::Instant::now();
    let summary = runner
        .run(&Stop::new(), &mut |event| {
            if let zajil_core::queue::QueueEvent::Download {
                event: zajil_core::Event::Progress { done, .. },
                ..
            } = event
                && done > 0
                && !lifted
            {
                lifted = true;
                steering
                    .update(|q| {
                        q.item_mut(id).unwrap().limit = None;
                        Ok(())
                    })
                    .unwrap();
            }
        })
        .await
        .unwrap();
    assert!(lifted);
    assert_eq!(summary.finished, 1);
    let took = started.elapsed();
    assert!(
        took < Duration::from_secs(16),
        "{took:?}: the new limit did not reach the running download"
    );
}
