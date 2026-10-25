//! Queues (M3): a number of downloads at once, steering a runner through
//! the queues file, and the file's own rules (ADR-0007).

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use zajil_core::queue::{QueueEvent, QueueStore, Runner, Status, StoreError};
use zajil_core::{Engine, Options, Stop};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

const MIB: usize = 1024 * 1024;

fn runner(store: &QueueStore) -> Runner {
    let engine = Engine::new(Options::default()).unwrap();
    Runner::new(engine, store.clone()).poll(Duration::from_millis(50))
}

fn set_max_active(store: &QueueStore, queue: &str, max_active: u32) {
    store
        .update(|queues| {
            queues.queue_or_new(queue)?.max_active = max_active;
            Ok(())
        })
        .unwrap();
}

fn add(store: &QueueStore, queue: &str, url: &str, dir: &Path) -> u64 {
    store.update(|queues| queues.add(queue, url, dir)).unwrap()
}

/// M3 success criterion: a queue of 10 downloads with 2 at once never runs
/// more than 2 at any moment. Counted twice: from the runner's events, and
/// by the server, which counts the bodies it sends at once.
#[tokio::test]
async fn ten_downloads_with_two_at_once_never_run_more_than_two() {
    let server = TestServer::http();
    let data = random_bytes(MIB, 400);
    // One connection each (too small to split), slow enough to overlap.
    server.put(
        "/f.bin",
        FileSpec::new(data.clone()).throttle(4 * MIB as u64),
    );
    let data_dir = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    set_max_active(&store, "main", 2);
    for _ in 0..10 {
        add(&store, "main", &server.url("/f.bin"), downloads.path());
    }

    let (mut now, mut most) = (0usize, 0usize);
    let summary = runner(&store)
        .run(&Stop::new(), &mut |event| match event {
            QueueEvent::Started { .. } => {
                now += 1;
                most = most.max(now);
            }
            QueueEvent::Finished { .. }
            | QueueEvent::Failed { .. }
            | QueueEvent::Stopped { .. } => now -= 1,
            QueueEvent::Download { .. }
            | QueueEvent::Waiting { .. }
            | QueueEvent::NeedsBrowser { .. } => {}
        })
        .await
        .unwrap();

    assert_eq!(summary.finished, 10, "{summary:?}");
    assert_eq!(most, 2);
    assert_eq!(server.most_concurrent("/f.bin"), 2);
    let queues = store.load().unwrap();
    let items = &queues.queue("main").unwrap().items;
    assert!(items.iter().all(|i| i.status == Status::Done), "{items:?}");
    let mut names: Vec<_> = fs::read_dir(downloads.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    names.sort();
    assert_eq!(names.len(), 10);
    for file in names {
        assert_eq!(sha256_hex(&fs::read(&file).unwrap()), sha256_hex(&data));
    }
}

/// Each queue has its own number: two queues of 1 run 2 downloads at once.
#[tokio::test]
async fn each_queue_has_its_own_number_of_downloads_at_once() {
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(MIB / 2, 401)).throttle(2 * MIB as u64),
    );
    let data_dir = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    for queue in ["a", "b"] {
        set_max_active(&store, queue, 1);
        for _ in 0..3 {
            add(&store, queue, &server.url("/f.bin"), downloads.path());
        }
    }
    let summary = runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    assert_eq!(summary.finished, 6);
    assert_eq!(server.most_concurrent("/f.bin"), 2);
}

/// Steering through the queues file, as `zajil queue pause` does from
/// another process: the paused download stops with its data recorded, and
/// resuming it continues where it was.
#[tokio::test]
async fn pausing_and_resuming_an_item_through_the_queues_file() {
    let server = TestServer::http();
    let data = random_bytes(4 * MIB, 402);
    server.put(
        "/f.bin",
        FileSpec::new(data.clone()).throttle(2 * MIB as u64),
    );
    let data_dir = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let id = add(&store, "main", &server.url("/f.bin"), downloads.path());

    let steering = store.clone();
    let mut paused = false;
    let mut resumed = false;
    let mut events = Vec::new();
    let summary = runner(&store)
        .run(&Stop::new(), &mut |event| {
            match &event {
                // Some data first, then pause it.
                QueueEvent::Download {
                    event: zajil_core::Event::Progress { done, .. },
                    ..
                } if *done > MIB as u64 && !paused => {
                    paused = true;
                    steering
                        .update(|q| {
                            q.item_mut(id).unwrap().status = Status::Paused;
                            Ok(())
                        })
                        .unwrap();
                }
                // Once it has stopped, it is paused, with its state file;
                // then resume it.
                QueueEvent::Stopped { item } if !resumed => {
                    assert_eq!(*item, id);
                    resumed = true;
                    steering
                        .update(|q| {
                            let item = q.item_mut(id).unwrap();
                            assert_eq!(item.status, Status::Paused);
                            assert!(item.state_file.as_ref().unwrap().exists());
                            item.status = Status::Waiting;
                            Ok(())
                        })
                        .unwrap();
                }
                _ => {}
            }
            events.push(event);
        })
        .await
        .unwrap();

    assert!(paused && resumed);
    assert_eq!(summary.finished, 1);
    let done = store.load().unwrap();
    let file = done.item(id).unwrap().file.clone().unwrap();
    assert_eq!(fs::read(file).unwrap(), data);
    // After the pause every request asked for a range: nothing started
    // over from byte 0.
    let gets = server.gets("/f.bin");
    let whole: Vec<_> = gets.iter().filter(|r| r.range.is_none()).collect();
    assert_eq!(whole.len(), 1, "{gets:?}");
}

/// Stopping the runner stops every download and waits for it: the items
/// go back to waiting, and the next run resumes them.
#[tokio::test]
async fn a_stopped_runner_leaves_its_downloads_waiting_and_resumable() {
    let server = TestServer::http();
    // Each connection 1 MiB/s, 8 MiB: no download can finish in the
    // moment between "both have more than 1 MiB" and the stop (on a fast
    // CI one sometimes did, with 4 MiB at 2 MiB/s).
    let data = random_bytes(8 * MIB, 403);
    server.put("/f.bin", FileSpec::new(data.clone()).throttle(MIB as u64));
    let data_dir = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    set_max_active(&store, "main", 2);
    let ids = [
        add(&store, "main", &server.url("/f.bin"), downloads.path()),
        add(&store, "main", &server.url("/f.bin"), downloads.path()),
    ];

    let stop = Stop::new();
    // Stop once both downloads have more than 1 MiB on disk.
    let mut far = std::collections::HashSet::new();
    let summary = runner(&store)
        .run(&stop.clone(), &mut |event| {
            if let QueueEvent::Download {
                item,
                event: zajil_core::Event::Progress { done, .. },
            } = event
                && done > MIB as u64
            {
                far.insert(item);
                if far.len() == 2 {
                    stop.request();
                }
            }
        })
        .await
        .unwrap();
    assert!(summary.stopped);

    let queues = store.load().unwrap();
    for id in ids {
        let item = queues.item(id).unwrap();
        assert_eq!(item.status, Status::Waiting, "{item:?}");
        assert!(item.state_file.as_ref().unwrap().exists(), "{item:?}");
    }

    let summary = runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    assert_eq!(summary.finished, 2);
    let queues = store.load().unwrap();
    for id in ids {
        let file = queues.item(id).unwrap().file.clone().unwrap();
        assert_eq!(fs::read(file).unwrap(), data);
    }
}

#[tokio::test]
async fn only_one_runner_runs_at_a_time() {
    let data_dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let _first = store.lock_runner().unwrap();
    let err = runner(&store).run(&Stop::new(), &mut |_| {}).await;
    assert!(matches!(err, Err(StoreError::RunnerBusy)), "{err:?}");
}

/// A failed download is marked failed, with its reason, and does not stop
/// the queue.
#[tokio::test]
async fn a_failed_item_does_not_stop_the_queue() {
    let server = TestServer::http();
    server.put("/ok.bin", FileSpec::new(random_bytes(1000, 404)));
    let data_dir = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    set_max_active(&store, "main", 1);
    let missing = add(&store, "main", &server.url("/missing"), downloads.path());
    let ok = add(&store, "main", &server.url("/ok.bin"), downloads.path());

    let summary = runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    assert_eq!((summary.finished, summary.failed), (1, 1));
    let queues = store.load().unwrap();
    let failed = queues.item(missing).unwrap();
    assert_eq!(failed.status, Status::Failed);
    assert!(
        failed.error.as_deref().unwrap().contains("404"),
        "{failed:?}"
    );
    assert_eq!(queues.item(ok).unwrap().status, Status::Done);
}

/// ADR-0007: writes from many processes (here threads) at once never lose
/// one: each one holds the lock from its read to the end of its write.
#[test]
fn concurrent_changes_are_never_lost() {
    let data_dir = tempfile::tempdir().unwrap();
    let store = Arc::new(QueueStore::new(data_dir.path()));
    let dir = data_dir.path().to_path_buf();
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let (store, dir) = (store.clone(), dir.clone());
            std::thread::spawn(move || {
                for i in 0..15 {
                    let url = format!("http://h/{t}/{i}");
                    store.update(|q| q.add("main", &url, &dir)).unwrap();
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let queues = store.load().unwrap();
    let items = &queues.queue("main").unwrap().items;
    assert_eq!(items.len(), 120);
    let mut ids: Vec<_> = items.iter().map(|i| i.id).collect();
    ids.dedup();
    assert_eq!(ids.len(), 120);
}

#[test]
fn the_queues_file_is_checked_and_a_damaged_one_is_never_overwritten() {
    let data_dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    add(&store, "main", "http://h/f.bin?sig=SECRET", data_dir.path());
    let path = store.path();
    // Only its owner may read it: a URL may hold a token (AGENTS.md §10.3).
    assert!(zajil_platform::fs::only_owner_can_access(&path).unwrap());
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"schema_version\": 1"), "{text}");
    assert!(text.contains("\"checksum\": \"sha256:"), "{text}");

    // A changed value with a stale checksum.
    let damaged = text.replace("http://h/f.bin", "http://h/g.bin");
    fs::write(&path, &damaged).unwrap();
    assert!(matches!(store.load(), Err(StoreError::Corrupt { .. })));
    let err = store.update(|q| q.add("main", "http://h/x", data_dir.path()));
    assert!(matches!(err, Err(StoreError::Corrupt { .. })), "{err:?}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        damaged,
        "left as it was"
    );

    for (bad, kind) in [
        ("{\"format\":\"other\"}", "other"),
        ("not json", "corrupt"),
        (
            &text.replace("\"schema_version\": 1", "\"schema_version\": 2") as &str,
            "version",
        ),
    ] {
        fs::write(&path, bad).unwrap();
        let err = store.load().unwrap_err();
        let matched = match kind {
            "other" => matches!(err, StoreError::NotAQueuesFile(_)),
            "version" => matches!(err, StoreError::UnsupportedVersion { version: 2, .. }),
            _ => matches!(err, StoreError::Corrupt { .. }),
        };
        assert!(matched, "{kind}: {err:?}");
    }
}

/// AGENTS.md §10.1: the queues file is never read through a link, and a
/// link planted at its temporary path is removed, not followed.
#[test]
fn links_at_the_queues_file_are_refused() {
    let data_dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    add(&store, "main", "http://h/f", data_dir.path());
    let target = outside.path().join("target.json");
    fs::copy(store.path(), &target).unwrap();
    let before = fs::read(&target).unwrap();

    // The temporary file is a link: the save removes it and writes anew.
    let tmp = data_dir.path().join("queues.json.tmp");
    zajil_platform::fs::symlink_file(&target, &tmp).expect("symbolic links must work here");
    add(&store, "main", "http://h/g", data_dir.path());
    assert_eq!(fs::read(&target).unwrap(), before);

    // The file itself is a symbolic link, then a hard link.
    fs::remove_file(store.path()).unwrap();
    zajil_platform::fs::symlink_file(&target, &store.path()).unwrap();
    assert!(matches!(store.load(), Err(StoreError::NotRegularFile(_))));
    fs::remove_file(store.path()).unwrap();
    fs::hard_link(&target, store.path()).unwrap();
    assert!(matches!(store.load(), Err(StoreError::NotRegularFile(_))));
    let err = store.update(|q| q.add("main", "http://h/x", data_dir.path()));
    assert!(err.is_err());
    assert_eq!(fs::read(&target).unwrap(), before);
}

/// Found by the CLI test of a killed runner: a reader that opened the file
/// just as a save replaced it saw a link count of 0 and refused it as a
/// hard link. Readers take no lock (ADR-0007), so this must always work.
#[test]
fn reading_while_another_process_saves_always_works() {
    let data_dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    add(&store, "main", "http://h/first", data_dir.path());
    let writer = {
        let (store, dir) = (store.clone(), data_dir.path().to_path_buf());
        std::thread::spawn(move || {
            for i in 0..150 {
                store
                    .update(|q| q.add("main", &format!("http://h/{i}"), &dir))
                    .unwrap();
            }
        })
    };
    let mut reads = 0;
    while !writer.is_finished() {
        store.load().unwrap();
        reads += 1;
    }
    writer.join().unwrap();
    assert!(reads > 150, "{reads}");
}

/// AGENTS.md §10.1: the lock files are never opened through a link either;
/// a link there is refused, and the file it points to is neither created
/// nor changed.
#[test]
fn links_at_the_lock_files_are_refused() {
    for name in ["queues.lock", "runner.lock"] {
        let data_dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data_dir.path());
        let missing = outside.path().join("not-created");
        zajil_platform::fs::symlink_file(&missing, &data_dir.path().join(name))
            .expect("symbolic links must work here");
        let err = if name == "runner.lock" {
            store.lock_runner().map(|_| ())
        } else {
            store
                .update(|q| q.add("main", "http://h/f", data_dir.path()))
                .map(|_| ())
        };
        assert!(
            matches!(err, Err(StoreError::NotRegularFile(_))),
            "{name}: {err:?}"
        );
        assert!(!missing.exists(), "{name}: created through the link");

        // A hard link to a file outside.
        fs::remove_file(data_dir.path().join(name)).unwrap();
        let target = outside.path().join("target");
        fs::write(&target, b"outside").unwrap();
        fs::hard_link(&target, data_dir.path().join(name)).unwrap();
        let err = if name == "runner.lock" {
            store.lock_runner().map(|_| ())
        } else {
            store
                .update(|q| q.add("main", "http://h/f", data_dir.path()))
                .map(|_| ())
        };
        assert!(
            matches!(err, Err(StoreError::NotRegularFile(_))),
            "{name}: {err:?}"
        );
        assert_eq!(fs::read(&target).unwrap(), b"outside");
    }
}

/// AGENTS.md §10.5: a stop requested at many moments, on one connection
/// and on four. Either nothing was written (no files left) or a state file
/// resumes to the right file at once, in the same process.
#[tokio::test]
async fn stopping_at_many_moments_leaves_a_resumable_download_or_nothing() {
    use zajil_core::Error;
    let server = TestServer::http();
    let data = random_bytes(4 * MIB, 405);
    server.put(
        "/f.bin",
        FileSpec::new(data.clone()).throttle(8 * MIB as u64),
    );
    for connections in [1, 4] {
        let engine = Engine::new(Options {
            connections,
            save_interval: Duration::from_millis(20),
            ..Options::default()
        })
        .unwrap();
        for ms in [0u64, 1, 3, 10, 30, 80, 150, 300] {
            let dir = tempfile::tempdir().unwrap();
            let stop = Stop::new();
            let later = stop.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                later.request();
            });
            let result = engine
                .download_until(&server.url("/f.bin"), dir.path(), &stop, &mut |_| {})
                .await;
            let file = match result {
                Err(Error::Stopped { state_file: None }) => {
                    let left = fs::read_dir(dir.path()).unwrap().count();
                    assert_eq!(left, 0, "{connections}, {ms} ms");
                    continue;
                }
                Err(Error::Stopped {
                    state_file: Some(state_file),
                }) => {
                    engine
                        .resume(&state_file, &mut |_| {})
                        .await
                        .unwrap_or_else(|e| panic!("{connections}, {ms} ms: {e}"))
                        .file
                }
                Ok(done) => done.file,
                Err(err) => panic!("{connections}, {ms} ms: {err}"),
            };
            assert_eq!(fs::read(&file).unwrap(), data, "{connections}, {ms} ms");
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }
}

/// Review of #31: an item kept the state file of a new download that had
/// failed before its first byte (its files were removed), and another
/// download of another URL then took that name; resuming the first item
/// finished the second's file and marked it done. Now a failure before any
/// byte forgets the state file, and a state file of another URL is never
/// resumed for an item.
#[tokio::test]
async fn an_item_never_resumes_another_downloads_state_file() {
    let server = TestServer::http();
    let (data_a, data_b) = (random_bytes(300_000, 420), random_bytes(300_000, 421));
    server.put(
        "/a/same.bin",
        FileSpec::new(data_a.clone()).cut_after(0, 100),
    );
    server.put(
        "/b/same.bin",
        FileSpec::new(data_b.clone()).cut_after(100_000, 1),
    );
    let data_dir = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    set_max_active(&store, "main", 1);
    let a = add(&store, "main", &server.url("/a/same.bin"), downloads.path());
    let b = add(&store, "main", &server.url("/b/same.bin"), downloads.path());
    let once = Engine::new(Options {
        max_failures: 1,
        ..Options::default()
    })
    .unwrap();
    let run = |store: &QueueStore| {
        Runner::new(once.clone(), store.clone()).poll(Duration::from_millis(20))
    };

    let summary = run(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    assert_eq!(summary.failed, 2);
    let queues = store.load().unwrap();
    assert_eq!(queues.item(a).unwrap().state_file, None, "A wrote nothing");
    let b_state = queues.item(b).unwrap().state_file.clone().unwrap();
    assert!(b_state.exists());

    // Even if A still pointed at B's state file (an older Zajil, or a
    // hand edit), it is not resumed: it is another URL.
    server.put("/a/same.bin", FileSpec::new(data_a.clone()));
    store
        .update(move |q| {
            let item = q.item_mut(a).unwrap();
            item.state_file = Some(b_state);
            item.status = Status::Waiting;
            Ok(())
        })
        .unwrap();
    let before_b = server.gets("/b/same.bin").len();
    let summary = run(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    assert_eq!(summary.finished, 1);
    assert_eq!(
        server.gets("/b/same.bin").len(),
        before_b,
        "B's file was asked for"
    );
    let queues = store.load().unwrap();
    let file = queues.item(a).unwrap().file.clone().unwrap();
    assert_eq!(fs::read(&file).unwrap(), data_a);
    // B's download is untouched and can still be resumed.
    assert_eq!(queues.item(b).unwrap().status, Status::Failed);
    assert!(
        queues
            .item(b)
            .unwrap()
            .state_file
            .as_ref()
            .unwrap()
            .exists()
    );
}

/// The number of connections a runner is given can change while it runs:
/// the next download takes the new number (the desktop's settings, M4).
/// One download at a time; the first runs with 2 connections, the setting
/// becomes 5 once its data flows, and the second uses 5. Counted by the
/// server.
#[tokio::test]
async fn a_changed_number_of_connections_applies_to_the_next_download() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let server = TestServer::http();
    // 8 MiB, slow per connection, so the segments overlap.
    let data = random_bytes(8 * MIB, 401);
    server.put(
        "/a.bin",
        FileSpec::new(data.clone()).throttle(4 * MIB as u64),
    );
    server.put(
        "/b.bin",
        FileSpec::new(data.clone()).throttle(4 * MIB as u64),
    );
    let data_dir = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    set_max_active(&store, "main", 1);
    add(&store, "main", &server.url("/a.bin"), downloads.path());
    add(&store, "main", &server.url("/b.bin"), downloads.path());

    let count = Arc::new(AtomicUsize::new(2));
    let setting = count.clone();
    let summary = runner(&store)
        .connections(count)
        .run(&Stop::new(), &mut |event| {
            // The first download's data flows: it has its connections.
            if let QueueEvent::Download { .. } = event {
                setting.store(5, Ordering::Relaxed);
            }
        })
        .await
        .unwrap();

    assert_eq!(summary.finished, 2, "{summary:?}");
    assert_eq!(server.most_concurrent("/a.bin"), 2);
    assert_eq!(server.most_concurrent("/b.bin"), 5);
}

/// M5: a download the extension sent with the browser's cookies and page
/// (`Runner::from_browser`) sends them; another item sends none; and the
/// item's entry is gone from memory once it has finished.
#[tokio::test]
async fn a_queued_download_sends_the_browser_s_headers_kept_for_it() {
    let server = TestServer::http();
    server.put("/with.bin", FileSpec::new(random_bytes(200_000, 91)));
    server.put("/without.bin", FileSpec::new(random_bytes(200_000, 92)));
    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data.path());
    let with = add(&store, "main", &server.url("/with.bin"), dir.path());
    add(&store, "main", &server.url("/without.bin"), dir.path());
    let headers = zajil_core::queue::FromBrowserById::default();
    let page = server.url("/page");
    headers.lock().unwrap().insert(
        with,
        zajil_core::FromBrowser::new(&server.url("/with.bin"), Some("sid=Q"), Some(&page)),
    );
    let summary = runner(&store)
        .from_browser(headers.clone())
        .run(&Stop::new(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(summary.finished, 2);
    for get in server.gets("/with.bin") {
        assert_eq!(get.cookie.as_deref(), Some("sid=Q"), "{get:?}");
        assert_eq!(get.referer.as_deref(), Some(page.as_str()));
    }
    for get in server.gets("/without.bin") {
        assert_eq!(
            (get.cookie.as_deref(), get.referer.as_deref()),
            (None, None)
        );
    }
    assert!(headers.lock().unwrap().is_empty(), "kept after it finished");
}

/// The headers of a failed download are kept for a retry, those of a
/// download removed from the queues are forgotten, and those of a download
/// added after the runner's last look are not (review of #76).
#[tokio::test]
async fn the_browser_s_headers_are_kept_on_failure_and_forgotten_on_removal() {
    let server = TestServer::http();
    server.put("/gone.bin", FileSpec::new(Vec::new()).forbid_after(0));
    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data.path());
    let failed = add(&store, "main", &server.url("/gone.bin"), dir.path());
    let headers = zajil_core::queue::FromBrowserById::default();
    let from = || zajil_core::FromBrowser::new(&server.url("/gone.bin"), Some("sid=Q"), None);
    headers.lock().unwrap().insert(failed, from());
    // An entry for an item that is no longer in the queues.
    let gone = add(&store, "main", &server.url("/gone.bin"), dir.path());
    store.update(|q| q.remove(gone).map(drop)).unwrap();
    headers.lock().unwrap().insert(gone, from());
    // An entry for an item the runner cannot have seen yet: the app adds
    // the item, then its headers, between a look and its sweep.
    let next = store.load().unwrap().next_id;
    headers.lock().unwrap().insert(next, from());
    let once = Engine::new(Options {
        max_failures: 1,
        ..Options::default()
    })
    .unwrap();
    let run = || {
        Runner::new(once.clone(), store.clone())
            .poll(Duration::from_millis(20))
            .from_browser(headers.clone())
    };
    let summary = run().run(&Stop::new(), &mut |_| {}).await.unwrap();
    assert_eq!(summary.failed, 1);
    let kept = || {
        let mut kept = headers.lock().unwrap().keys().copied().collect::<Vec<_>>();
        kept.sort();
        kept
    };
    assert_eq!(
        kept(),
        [failed, next],
        "the failed item's entry stays, and the one not seen yet; the gone one goes"
    );
    store.update(|q| q.remove(failed).map(drop)).unwrap();
    run().run(&Stop::new(), &mut |_| {}).await.unwrap();
    assert_eq!(kept(), [next], "kept after the item was removed");
}

fn from_browser(cookies: bool, referrer: Option<&str>) -> zajil_core::queue::BrowserSource {
    zajil_core::queue::BrowserSource {
        instance: "3f2b8c1e-0000-4000-8000-000000000001".into(),
        store: Some("firefox-container-1".into()),
        cookies,
        referrer: referrer.map(str::to_owned),
        ticket: Some("ticket-1".into()),
    }
}

/// After a restart (M5): a download that came from the browser with its
/// cookies, and has none in memory, waits for that browser and takes no
/// room in its queue; once the cookies are given again it goes, with them;
/// and when it is done, where it came from is forgotten.
#[tokio::test]
async fn a_download_waits_for_the_browser_s_cookies_after_a_restart() {
    let server = TestServer::http();
    server.put("/signed.bin", FileSpec::new(random_bytes(200_000, 93)));
    server.put("/plain.bin", FileSpec::new(random_bytes(200_000, 94)));
    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data.path());
    let signed = add(&store, "main", &server.url("/signed.bin"), dir.path());
    let plain = add(&store, "main", &server.url("/plain.bin"), dir.path());
    let page = server.url("/page");
    // One at a time: the waiting one must leave its room to the other.
    store
        .update(|q| {
            q.queue_mut("main").unwrap().max_active = 1;
            q.item_mut(signed).unwrap().browser = Some(from_browser(true, Some(&page)));
            Ok(())
        })
        .unwrap();
    // Nothing in memory: as after a restart.
    let headers = zajil_core::queue::FromBrowserById::default();
    let stop = Stop::new();
    let mut asked = Vec::new();
    {
        let (store, stop) = (store.clone(), stop.clone());
        runner(&store)
            .from_browser(headers.clone())
            .run(&stop.clone(), &mut |event| {
                if let QueueEvent::NeedsBrowser { item, url, source } = event {
                    assert_eq!(url, server.url("/signed.bin"));
                    asked.push((item, source));
                    if store.load().unwrap().item(plain).unwrap().status == Status::Done {
                        stop.request();
                    }
                }
            })
            .await
            .unwrap();
    }
    let queues = store.load().unwrap();
    assert_eq!(
        queues.item(plain).unwrap().status,
        Status::Done,
        "it took the room"
    );
    assert_eq!(queues.item(signed).unwrap().status, Status::Waiting);
    assert!(
        server.gets("/signed.bin").is_empty(),
        "went without its cookies"
    );
    assert_eq!(asked[0], (signed, from_browser(true, Some(&page))));

    // The browser gives them again.
    headers.lock().unwrap().insert(
        signed,
        zajil_core::FromBrowser::new(&server.url("/signed.bin"), Some("sid=AGAIN"), Some(&page)),
    );
    runner(&store)
        .from_browser(headers.clone())
        .run(&Stop::new(), &mut |_| {})
        .await
        .unwrap();
    let item = store.load().unwrap().item(signed).unwrap().clone();
    assert_eq!(item.status, Status::Done);
    assert_eq!(item.browser, None, "kept after it was done");
    for get in server.gets("/signed.bin") {
        assert_eq!(get.cookie.as_deref(), Some("sid=AGAIN"), "{get:?}");
        assert_eq!(get.referer.as_deref(), Some(page.as_str()));
    }
}

/// A download from the browser without cookies does not wait: after a
/// restart it goes at once, with its page from the queues file. And the
/// file never holds cookies.
#[tokio::test]
async fn a_download_from_the_browser_without_cookies_goes_with_its_page() {
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(random_bytes(200_000, 95)));
    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data.path());
    let id = add(&store, "main", &server.url("/f.bin"), dir.path());
    let page = server.url("/page");
    store
        .update(|q| {
            q.item_mut(id).unwrap().browser = Some(from_browser(false, Some(&page)));
            Ok(())
        })
        .unwrap();
    let text = fs::read_to_string(data.path().join("queues.json")).unwrap();
    assert!(
        text.contains("\"instance\"") && !text.to_lowercase().contains("cookie\":\""),
        "{text}"
    );
    runner(&store)
        .from_browser(Default::default())
        .run(&Stop::new(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(store.load().unwrap().item(id).unwrap().status, Status::Done);
    for get in server.gets("/f.bin") {
        assert_eq!(
            (get.cookie.as_deref(), get.referer.as_deref()),
            (None, Some(page.as_str()))
        );
    }
}

/// The page a download came from is never printed (owner's decision on
/// #80), not even in a debug message.
#[test]
fn the_browser_s_page_is_never_printed() {
    let source = from_browser(true, Some("https://example.com/secret-page?token=T"));
    let shown = format!("{source:?}");
    assert!(
        !shown.contains("secret-page") && !shown.contains("token"),
        "{shown}"
    );
}

/// Security review #84: a download held for the user's OK does not start
/// while paused, and its hold is cleared once the user starts it.
#[tokio::test]
async fn a_held_download_starts_only_when_the_user_starts_it() {
    let server = TestServer::http();
    server.put("/setup.exe", FileSpec::new(random_bytes(50_000, 96)));
    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data.path());
    let id = add(&store, "main", &server.url("/setup.exe"), dir.path());
    store
        .update(|q| {
            let item = q.item_mut(id).unwrap();
            item.status = Status::Paused;
            item.needs_ok = true;
            Ok(())
        })
        .unwrap();
    runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    assert!(server.gets("/setup.exe").is_empty(), "it started by itself");
    store
        .update(|q| {
            q.item_mut(id).unwrap().status = Status::Waiting;
            Ok(())
        })
        .unwrap();
    runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    let item = store.load().unwrap().item(id).unwrap().clone();
    assert_eq!((item.status, item.needs_ok), (Status::Done, false));
}

/// Review of #90: a server that names another file at the download itself
/// than at the check (`a.zip`, then `setup.exe`) cannot start a file that
/// runs code: a download from a page is held by the name the download
/// gets, before anything is made; once the user starts it, it goes.
#[tokio::test]
async fn a_download_from_a_page_is_held_by_the_name_the_download_itself_gets() {
    let server = TestServer::http();
    let data = random_bytes(50_000, 98);
    server.put(
        "/get",
        FileSpec::new(data.clone())
            .content_disposition("attachment; filename=\"a.zip\"")
            .replaced_after(
                1,
                FileSpec::new(data.clone())
                    .content_disposition("attachment; filename=\"setup.exe\""),
            ),
    );
    // The check, as the app makes it: it is told `a.zip`.
    let engine = Engine::new(Options::default()).unwrap();
    assert_eq!(
        engine.probe(&server.url("/get")).await.unwrap().name,
        "a.zip"
    );
    let data_dir = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let id = add(&store, "main", &server.url("/get"), dir.path());
    store
        .update(|q| {
            q.item_mut(id).unwrap().from_page = true;
            Ok(())
        })
        .unwrap();
    let mut failed = false;
    runner(&store)
        .run(&Stop::new(), &mut |event| {
            failed |= matches!(event, QueueEvent::Failed { .. })
        })
        .await
        .unwrap();
    let item = store.load().unwrap().item(id).unwrap().clone();
    assert_eq!(
        (item.status, item.needs_ok),
        (Status::Paused, true),
        "{item:?}"
    );
    assert!(!failed, "a hold is not a failure");
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        0,
        "a file was made"
    );
    // The user starts it: it goes, and is not held again.
    store
        .update(|q| {
            q.item_mut(id).unwrap().status = Status::Waiting;
            Ok(())
        })
        .unwrap();
    runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    let item = store.load().unwrap().item(id).unwrap().clone();
    assert_eq!(item.status, Status::Done, "{item:?}");
    assert!(item.ok_given);
    assert_eq!(fs::read(dir.path().join("setup.exe")).unwrap(), data);
/// Security review #84: a private window's download leaves nothing in the
/// queues once it is done; only its file.
#[tokio::test]
async fn a_private_window_s_download_leaves_nothing_once_done() {
    let server = TestServer::http();
    let data_bytes = random_bytes(50_000, 97);
    server.put("/private.bin", FileSpec::new(data_bytes.clone()));
    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data.path());
    let id = add(&store, "main", &server.url("/private.bin"), dir.path());
    let other = add(&store, "main", &server.url("/private.bin"), dir.path());
    store
        .update(|q| {
            q.item_mut(id).unwrap().private = true;
            Ok(())
        })
        .unwrap();
    let mut finished = Vec::new();
    runner(&store)
        .run(&Stop::new(), &mut |event| {
            if let QueueEvent::Finished { item, .. } = event {
                finished.push(item);
            }
        })
        .await
        .unwrap();
    finished.sort();
    assert_eq!(finished, [id, other]);
    let queues = store.load().unwrap();
    assert!(queues.item(id).is_none(), "the private one stayed");
    assert_eq!(queues.item(other).unwrap().status, Status::Done);
    let text = fs::read_to_string(data.path().join("queues.json")).unwrap();
    assert_eq!(text.matches("private.bin").count(), 1, "{text}");
    assert_eq!(
        fs::read(dir.path().join("private.bin")).unwrap(),
        data_bytes
    );
}
