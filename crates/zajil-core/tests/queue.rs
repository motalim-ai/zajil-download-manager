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
            QueueEvent::Download { .. } => {}
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
    let data = random_bytes(4 * MIB, 403);
    server.put(
        "/f.bin",
        FileSpec::new(data.clone()).throttle(2 * MIB as u64),
    );
    let data_dir = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    set_max_active(&store, "main", 2);
    let ids = [
        add(&store, "main", &server.url("/f.bin"), downloads.path()),
        add(&store, "main", &server.url("/f.bin"), downloads.path()),
    ];

    let stop = Stop::new();
    let mut started = 0;
    let summary = runner(&store)
        .run(&stop.clone(), &mut |event| {
            if let QueueEvent::Download {
                event: zajil_core::Event::Progress { done, .. },
                ..
            } = event
                && done > MIB as u64
            {
                started += 1;
                if started == 4 {
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
