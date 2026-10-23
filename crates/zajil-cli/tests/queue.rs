//! `zajil queue` (M3): managing queues from the command line, and a
//! runner killed without warning.

mod common;

use std::fs;
use std::path::Path;
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use zajil_core::queue::{QueueStore, Status};
use zajil_core::state::DownloadState;
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

const MIB: usize = 1024 * 1024;

fn zajil(data: &Path, args: &[&str]) -> Output {
    common::zajil_command()
        .arg("--data-dir")
        .arg(data)
        .args(args)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn printed(out: &Output) -> String {
    format!("{}{}", text(&out.stdout), text(&out.stderr))
}

/// M3 success criterion: `zajil queue run` killed without warning, then run
/// again. The queues, the progress and the settings are as they were, and
/// the downloads resume (none starts over).
#[test]
fn a_killed_queue_run_keeps_its_queues_progress_and_settings() {
    let server = TestServer::http();
    let files: Vec<(String, Vec<u8>)> = ["a", "b", "c"]
        .iter()
        .enumerate()
        .map(|(i, name)| {
            (
                format!("/{name}.bin"),
                random_bytes(8 * MIB, 600 + i as u64),
            )
        })
        .collect();
    let spec =
        |data: &Vec<u8>, path: &str| FileSpec::new(data.clone()).etag(Some(&format!("\"{path}\"")));
    for (path, data) in &files {
        server.put(path, spec(data, path).throttle(MIB as u64));
    }
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let dir = downloads.path().to_str().unwrap();

    let out = zajil(data.path(), &["queue", "set", "--max-active", "2"]);
    assert!(out.status.success(), "{}", printed(&out));
    let urls: Vec<String> = files.iter().map(|(path, _)| server.url(path)).collect();
    let mut add = vec!["queue", "add", "--dir", dir];
    add.extend(urls.iter().map(String::as_str));
    let out = zajil(data.path(), &add);
    assert!(out.status.success(), "{}", printed(&out));

    let mut child = common::zajil_command()
        .arg("--data-dir")
        .arg(data.path())
        .args(["--connections", "2", "queue", "run"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // Wait until both running downloads have durable progress, then kill.
    let store = QueueStore::new(data.path());
    let deadline = Instant::now() + Duration::from_secs(60);
    let done_before = loop {
        assert!(Instant::now() < deadline, "no progress was saved");
        assert!(child.try_wait().unwrap().is_none(), "zajil exited early");
        let done: Vec<u64> = store
            .load()
            .unwrap()
            .queue("main")
            .unwrap()
            .items
            .iter()
            .filter_map(|item| item.state_file.as_deref())
            .filter_map(|path| DownloadState::load(path).ok())
            .map(|state| state.bytes_done())
            .filter(|&done| done >= MIB as u64)
            .collect();
        if done.len() == 2 {
            break done;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    child.kill().unwrap();
    child.wait().unwrap();
    let requests_before: Vec<usize> = files
        .iter()
        .map(|(path, _)| server.gets(path).len())
        .collect();

    // Everything is still there: the setting, the three downloads in
    // order, and the progress of the two that ran.
    let queues = store.load().unwrap();
    let queue = queues.queue("main").unwrap();
    assert_eq!(queue.max_active, 2);
    let urls_kept: Vec<&str> = queue.items.iter().map(|i| i.url.as_str()).collect();
    assert_eq!(urls_kept, urls);
    assert_eq!(queue.items[2].status, Status::Waiting);
    assert!(queue.items[2].state_file.is_none());
    let out = zajil(data.path(), &["queue", "list"]);
    let list = printed(&out);
    assert!(
        list.contains("Queue main (2 at once, 3 downloads)"),
        "{list}"
    );
    assert!(list.contains("a.bin") && list.contains("b.bin"), "{list}");
    assert!(done_before.iter().all(|&d| d < 8 * MIB as u64));

    // The server is fast now (same version of each file).
    for (path, data) in &files {
        server.put(path, spec(data, path));
    }
    let out = common::zajil_command()
        .arg("--data-dir")
        .arg(data.path())
        .args(["--connections", "2", "queue", "run"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", printed(&out));
    let resumed = printed(&out);
    assert_eq!(resumed.matches("resuming").count(), 2, "{resumed}");

    let queues = store.load().unwrap();
    for (item, (path, data)) in queues.queue("main").unwrap().items.iter().zip(&files) {
        assert_eq!(item.status, Status::Done, "{item:?}");
        let file = item.file.as_deref().unwrap();
        assert_eq!(sha256_hex(&fs::read(file).unwrap()), sha256_hex(data));
        assert_eq!(file.file_name().unwrap().to_str().unwrap(), &path[1..]);
    }
    // The two that had progress went on with ranges: no request of the
    // second run asked for a whole file.
    for (i, (path, _)) in files.iter().enumerate().take(2) {
        let after = &server.gets(path)[requests_before[i]..];
        assert!(!after.is_empty());
        assert!(after.iter().all(|r| r.range.is_some()), "{after:?}");
    }
    // Nothing is left but the three files.
    let names = fs::read_dir(downloads.path()).unwrap().count();
    assert_eq!(names, 3);
}

#[test]
fn queue_commands_change_the_queues_file() {
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let dir = downloads.path().to_str().unwrap();

    let out = zajil(
        data.path(),
        &[
            "queue",
            "add",
            "--dir",
            dir,
            "http://127.0.0.1:9/a.bin?sig=SECRET-TOKEN",
            "http://127.0.0.1:9/b.bin",
        ],
    );
    assert!(out.status.success(), "{}", printed(&out));
    assert!(!printed(&out).contains("SECRET-TOKEN"), "{}", printed(&out));

    // Never with a user name or password: they would have to be saved.
    let out = zajil(data.path(), &["queue", "add", "http://u:S3cret@h/x"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(!printed(&out).contains("S3cret"));

    for args in [
        &["queue", "set", "--max-active", "4", "--queue", "night"][..],
        &[
            "queue",
            "add",
            "--queue",
            "night",
            "--dir",
            dir,
            "http://h/c",
        ],
        &["queue", "pause", "1"],
        &["queue", "remove", "2"],
    ] {
        let out = zajil(data.path(), args);
        assert!(out.status.success(), "{args:?}: {}", printed(&out));
    }
    let out = zajil(data.path(), &["queue", "list"]);
    let list = printed(&out);
    assert!(!list.contains("SECRET-TOKEN"), "{list}");
    assert!(
        list.contains("Queue main (3 at once, 1 downloads)"),
        "{list}"
    );
    assert!(
        list.contains("Queue night (4 at once, 1 downloads)"),
        "{list}"
    );
    assert!(list.contains("paused"), "{list}");
    assert!(!list.contains("b.bin"), "{list}");

    let store = QueueStore::new(data.path());
    let queues = store.load().unwrap();
    assert_eq!(queues.item(1).unwrap().status, Status::Paused);
    assert!(queues.item(2).is_none());
    // The token is kept for the download, in a file only its owner reads.
    assert!(queues.item(1).unwrap().url.contains("SECRET-TOKEN"));
    assert!(zajil_platform::fs::only_owner_can_access(&store.path()).unwrap());

    let out = zajil(data.path(), &["queue", "resume", "1"]);
    assert!(printed(&out).contains("1 is waiting"), "{}", printed(&out));
    for (args, code) in [
        (&["queue", "pause", "99"][..], 1),
        (&["queue", "set", "--max-active", "0"], 2),
        (&["queue", "set", "--max-active", "33"], 2),
        (&["queue", "set", "--queue", "a b"], 1),
    ] {
        let out = zajil(data.path(), args);
        assert_eq!(out.status.code(), Some(code), "{args:?}: {}", printed(&out));
    }
}

#[test]
fn queue_run_downloads_everything_and_reports_failures() {
    let server = TestServer::http();
    let data_a = random_bytes(300_000, 610);
    server.put("/a.bin", FileSpec::new(data_a.clone()));
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let dir = downloads.path().to_str().unwrap();
    let missing = server.url("/missing.bin");
    let out = zajil(
        data.path(),
        &[
            "queue",
            "add",
            "--dir",
            dir,
            &server.url("/a.bin"),
            &missing,
        ],
    );
    assert!(out.status.success());

    let out = zajil(data.path(), &["--tries", "1", "queue", "run"]);
    assert_eq!(out.status.code(), Some(1), "{}", printed(&out));
    let shown = printed(&out);
    assert!(shown.contains("[1] saved"), "{shown}");
    assert!(shown.contains("[2] failed"), "{shown}");
    assert_eq!(fs::read(downloads.path().join("a.bin")).unwrap(), data_a);
    let list = printed(&zajil(data.path(), &["queue", "list"]));
    assert!(list.contains("done") && list.contains("failed"), "{list}");
    assert!(list.contains("404"), "{list}");
}

/// The one test on the real clock (owner's decision on the M3 plan): every
/// other limit and schedule test runs on a clock the test owns. This one
/// checks that both reach `zajil queue run` through the system clock and
/// time zone:
/// - a queue scheduled to start 3 seconds from now (local time) starts
///   then, not before, and within 2 seconds (generous for a slow CI);
/// - the program-wide limit of 1 MiB/s set with `zajil queue speed-limit`
///   makes its 3 MiB take at least about 2.7 s. Only that lower bound is
///   checked: a slow CI can only make it longer.
#[test]
fn the_real_clock_drives_limits_and_schedules() {
    use std::io::{BufRead, BufReader};

    let server = TestServer::http();
    let data = random_bytes(3 * MIB / 2, 620);
    server.put("/f.bin", FileSpec::new(data.clone()));
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let dir = downloads.path().to_str().unwrap();
    // Whole seconds, as the schedule is.
    let now = jiff::Zoned::now();
    let start = now
        .round(
            jiff::ZonedRound::new()
                .smallest(jiff::Unit::Second)
                .mode(jiff::RoundMode::Ceil),
        )
        .unwrap()
        .checked_add(jiff::SignedDuration::from_secs(3))
        .unwrap();
    let stop = start
        .checked_add(jiff::SignedDuration::from_hours(1))
        .unwrap();
    let (start_text, stop_text) = (
        start.strftime("%H:%M:%S").to_string(),
        stop.strftime("%H:%M:%S").to_string(),
    );
    for args in [
        &["queue", "speed-limit", "1M"][..],
        &["queue", "set", "--max-active", "2"],
        &[
            "queue",
            "schedule",
            "--start",
            &start_text,
            "--stop",
            &stop_text,
        ],
        &[
            "queue",
            "add",
            "--dir",
            dir,
            &server.url("/f.bin"),
            &server.url("/f.bin"),
        ],
    ] {
        let out = zajil(state.path(), args);
        assert!(out.status.success(), "{args:?}: {}", printed(&out));
    }
    let list = printed(&zajil(state.path(), &["queue", "list"]));
    assert!(
        list.contains("Speed limit for all downloads: 1.0 MiB/s"),
        "{list}"
    );
    assert!(list.contains(&format!("runs from {start_text}")), "{list}");

    let mut child = common::zajil_command()
        .arg("--data-dir")
        .arg(state.path())
        .args(["queue", "run"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Each line of the runner, with the time it arrived.
    let lines: Vec<(jiff::Timestamp, String)> = BufReader::new(child.stderr.take().unwrap())
        .lines()
        .map(|line| (jiff::Timestamp::now(), line.unwrap()))
        .collect();
    let ended = jiff::Timestamp::now();
    assert!(child.wait().unwrap().success(), "{lines:?}");

    let waits = lines
        .iter()
        .any(|(_, l)| l.contains("waits for its schedule"));
    assert!(waits, "{lines:?}");
    let started = lines
        .iter()
        .find(|(_, l)| l.starts_with("[1] starting"))
        .map(|(at, _)| *at)
        .unwrap_or_else(|| panic!("{lines:?}"));
    let late = started.duration_since(start.timestamp());
    assert!(
        late >= jiff::SignedDuration::ZERO && late <= jiff::SignedDuration::from_secs(2),
        "started {late:?} after {start_text}"
    );
    let took = ended.duration_since(started);
    assert!(took >= jiff::SignedDuration::from_millis(2600), "{took:?}");
    for entry in fs::read_dir(downloads.path()).unwrap() {
        assert_eq!(fs::read(entry.unwrap().path()).unwrap(), data);
    }
}

#[test]
fn schedules_are_checked_and_can_be_removed() {
    let data = tempfile::tempdir().unwrap();
    for (args, code) in [
        (&["queue", "schedule", "--start", "25:00"][..], 2),
        (&["queue", "schedule", "--start", "7:00"], 2),
        (&["queue", "schedule", "--days", "funday"], 2),
        (&["queue", "schedule"], 1),
        (&["queue", "schedule", "--none", "--start", "07:00"], 2),
        (
            &[
                "queue", "schedule", "--start", "23:00", "--stop", "07:00", "--days", "fri,sat",
            ],
            0,
        ),
    ] {
        let out = zajil(data.path(), args);
        assert_eq!(out.status.code(), Some(code), "{args:?}: {}", printed(&out));
    }
    let list = printed(&zajil(data.path(), &["queue", "list"]));
    assert!(
        list.contains("runs from 23:00 to 07:00, on fri,sat"),
        "{list}"
    );
    let out = zajil(data.path(), &["queue", "schedule", "--none"]);
    assert!(
        printed(&out).contains("runs at any time"),
        "{}",
        printed(&out)
    );
    let store = QueueStore::new(data.path());
    assert!(
        store
            .load()
            .unwrap()
            .queue("main")
            .unwrap()
            .schedule
            .is_none()
    );
}

#[test]
fn speed_limits_are_checked_and_can_be_removed() {
    let data = tempfile::tempdir().unwrap();
    for (args, code) in [
        (&["queue", "speed-limit", "fast"][..], 2),
        (&["queue", "speed-limit", "0"], 2),
        (&["get", "http://h/f", "--limit", "1X"], 2),
        (&["queue", "add", "http://h/f", "--limit", "-1"], 2),
        (&["queue", "speed-limit", "500K"], 0),
        (&["queue", "add", "http://h/f", "--limit", "1.5M"], 0),
    ] {
        let out = zajil(data.path(), args);
        assert_eq!(out.status.code(), Some(code), "{args:?}: {}", printed(&out));
    }
    let list = printed(&zajil(data.path(), &["queue", "list"]));
    assert!(
        list.contains("Speed limit for all downloads: 500.0 KiB/s"),
        "{list}"
    );
    assert!(list.contains("(limit 1.5 MiB/s)"), "{list}");
    let out = zajil(data.path(), &["queue", "speed-limit", "none"]);
    assert!(
        printed(&out).contains("No speed limit"),
        "{}",
        printed(&out)
    );
    let store = QueueStore::new(data.path());
    assert_eq!(store.load().unwrap().settings.speed_limit, None);
}
