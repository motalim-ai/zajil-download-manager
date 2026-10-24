//! `zajil category` and categories in `zajil get` and `zajil queue` (M4),
//! and `zajil queue export` / `import`.

mod common;

use std::fs;
use std::path::Path;
use std::process::Output;
use std::time::{Duration, Instant};

use zajil_core::category::CategoryStore;
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

fn printed(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn ok(data: &Path, args: &[&str]) -> String {
    let out = zajil(data, args);
    assert!(out.status.success(), "{args:?}: {}", printed(&out));
    printed(&out)
}

fn path(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// M4 success criterion: a `.zip` goes to the archives category
/// («مضغوطات»), and a rule the user adds still applies after a restart.
/// Here the restart is a `zajil queue run` killed without warning in the
/// middle of a download, then run again: the rule is read from
/// `categories.json`, not from memory.
#[test]
fn a_zip_goes_to_archives_and_a_users_rule_survives_a_killed_run() {
    let server = TestServer::http();
    let slow = random_bytes(6 * MIB, 801);
    let zip = random_bytes(300_000, 802);
    let cbz = random_bytes(200_000, 803);
    server.put(
        "/slow.bin",
        FileSpec::new(slow.clone())
            .etag(Some("\"slow\""))
            .throttle(MIB as u64),
    );
    server.put("/photos.zip", FileSpec::new(zip.clone()));
    server.put("/comic.cbz", FileSpec::new(cbz.clone()));
    let data = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let base = home.path().join("downloads");
    let archives = home.path().join("مضغوطات");
    let comics = home.path().join("Comics");

    ok(
        data.path(),
        &["category", "set", "archives", "--folder", path(&archives)],
    );
    ok(
        data.path(),
        &["category", "add", "comics", "--name", "قصص مصورة"],
    );
    ok(
        data.path(),
        &[
            "category",
            "set",
            "comics",
            "--add-ext",
            ".CBZ",
            "--folder",
            path(&comics),
        ],
    );
    let which = ok(data.path(), &["category", "which", "my photos.ZIP"]);
    assert!(which.contains("(archives)"), "{which}");

    // One at a time, so the kill comes during the slow download, before
    // the two others started.
    ok(data.path(), &["queue", "set", "--max-active", "1"]);
    ok(
        data.path(),
        &[
            "queue",
            "add",
            "--dir",
            path(&base),
            "--by-category",
            &server.url("/slow.bin"),
            &server.url("/photos.zip"),
            &server.url("/comic.cbz"),
        ],
    );
    // What zajil prints is kept, and shown if the test fails, so a failure
    // on CI says why (owner's review of #41).
    let out_file = home.path().join("run-1.out");
    let err_file = home.path().join("run-1.err");
    // One connection: the server limits each connection to 1 MiB/s, so
    // the 6 MiB take about 6 s and progress is saved (once a second)
    // before the kill. With the default 8 connections they took under a
    // second, so zajil finished everything (exit code 0, "3 finished;
    // nothing is left to download") before any progress was saved, and
    // the test failed with "zajil exited early" about one run in 20 here
    // and on Linux and macOS CI (review of #41).
    let mut child = common::zajil_command()
        .arg("--data-dir")
        .arg(data.path())
        .args(["--connections", "1", "queue", "run"])
        .stdout(fs::File::create(&out_file).unwrap())
        .stderr(fs::File::create(&err_file).unwrap())
        .spawn()
        .unwrap();
    let shown = || {
        format!(
            "stdout:\n{}\nstderr:\n{}",
            fs::read_to_string(&out_file).unwrap_or_default(),
            fs::read_to_string(&err_file).unwrap_or_default()
        )
    };
    let store = QueueStore::new(data.path());
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(
            Instant::now() < deadline,
            "no progress was saved\n{}",
            shown()
        );
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "zajil exited early ({status}); queues: {:?}\n{}",
                store.load().unwrap().queue("main").unwrap().items,
                shown()
            );
        }
        let saved = store.load().unwrap().queue("main").unwrap().items[0]
            .state_file
            .as_deref()
            .and_then(|p| DownloadState::load(p).ok())
            .is_some_and(|s| s.bytes_done() >= MIB as u64);
        if saved {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.kill().unwrap();
    child.wait().unwrap();

    // The rule is on disk, as the user left it.
    let categories = CategoryStore::new(data.path()).load().unwrap();
    assert_eq!(categories.classify("x.cbz", None).unwrap().id, "comics");

    // Run again, as after a restart: everything finishes, each file in its
    // category's folder, and the file of no category in the given folder.
    let out = zajil(data.path(), &["queue", "run"]);
    assert!(out.status.success(), "{}", printed(&out));
    let queues = store.load().unwrap();
    assert!(
        queues
            .queue("main")
            .unwrap()
            .items
            .iter()
            .all(|i| i.status == Status::Done),
        "{queues:?}"
    );
    for (folder, name, data) in [
        (&base, "slow.bin", &slow),
        (&archives, "photos.zip", &zip),
        (&comics, "comic.cbz", &cbz),
    ] {
        let file = folder.join(name);
        assert_eq!(
            sha256_hex(&fs::read(&file).unwrap()),
            sha256_hex(data),
            "{}",
            file.display()
        );
    }
    assert_eq!(fs::read_dir(&archives).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&comics).unwrap().count(), 1);
}

#[test]
fn zajil_get_uses_the_category_and_a_folder_given_alone_wins() {
    let server = TestServer::http();
    server.put("/a.zip", FileSpec::new(random_bytes(10_000, 810)));
    let data = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let archives = home.path().join("archives");
    let given = home.path().join("given");
    ok(
        data.path(),
        &["category", "set", "archives", "--folder", path(&archives)],
    );
    ok(
        data.path(),
        &[
            "get",
            "--dir",
            path(&given),
            "--by-category",
            &server.url("/a.zip"),
        ],
    );
    assert!(archives.join("a.zip").is_file());
    ok(
        data.path(),
        &["get", "--dir", path(&given), &server.url("/a.zip")],
    );
    assert!(given.join("a.zip").is_file());
}

#[test]
fn bad_category_input_is_refused_clearly() {
    let data = tempfile::tempdir().unwrap();
    for (args, expect) in [
        (
            vec!["category", "add", "Bad Id", "--name", "x"],
            "invalid category id",
        ),
        (
            vec!["category", "add", "x", "--name", "a\u{202e}b"],
            "invalid category name",
        ),
        (
            vec![
                "category",
                "set",
                "archives",
                "--add-site",
                "user@example.com",
            ],
            "invalid site",
        ),
        (
            vec!["category", "set", "archives", "--add-ext", "z/ip"],
            "invalid extension",
        ),
        (
            vec!["category", "set", "nothing", "--add-ext", "zip"],
            "there is no category",
        ),
        (
            vec!["category", "add", "archives", "--name", "x"],
            "already a category",
        ),
    ] {
        let out = zajil(data.path(), &args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", printed(&out));
        assert!(
            printed(&out).contains(expect),
            "{args:?}: {}",
            printed(&out)
        );
    }
}

/// The export and import commands: the round trip, a file only its owner
/// can read, no token printed, and a hostile file refused with exit code 1
/// (not a crash) while the queues stay as they were.
#[test]
fn export_then_import_through_the_command_line() {
    let data = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let file = home.path().join("queues export.json");
    let given = home.path().join("d");
    ok(
        data.path(),
        &[
            "queue",
            "add",
            "--dir",
            path(&given),
            "https://cdn.example.net/b.zip?sig=SECRET-TOKEN",
            "http://example.org/c.bin",
        ],
    );
    ok(
        data.path(),
        &["queue", "set", "--queue", "ليلي", "--max-active", "2"],
    );
    // `main` with settings of its own: the new data folder's default `main`
    // must take them (review of #41).
    ok(data.path(), &["queue", "set", "--max-active", "1"]);
    ok(
        data.path(),
        &["queue", "schedule", "--start", "02:00", "--stop", "06:00"],
    );
    let out = ok(data.path(), &["queue", "export", path(&file)]);
    assert!(!out.contains("SECRET-TOKEN"), "{out}");
    assert!(
        out.contains("some may contain temporary access keys. Do not share it."),
        "{out}"
    );
    assert!(zajil_platform::fs::only_owner_can_access(&file).unwrap());
    let again = zajil(data.path(), &["queue", "export", path(&file)]);
    assert_eq!(again.status.code(), Some(1));
    assert!(
        printed(&again).contains("already exists"),
        "{}",
        printed(&again)
    );

    let out = ok(
        elsewhere.path(),
        &["queue", "import", path(&file), "--keep-folders"],
    );
    assert!(!out.contains("SECRET-TOKEN"), "{out}");
    let before = QueueStore::new(data.path()).load().unwrap();
    let after = QueueStore::new(elsewhere.path()).load().unwrap();
    let names = |q: &zajil_core::queue::QueuesFile| {
        q.queues
            .iter()
            .map(|q| {
                (
                    q.name.clone(),
                    q.max_active,
                    q.items
                        .iter()
                        .map(|i| (i.url.clone(), i.dir.clone()))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&after), names(&before));
    let schedules = |q: &zajil_core::queue::QueuesFile| {
        q.queues
            .iter()
            .map(|q| q.schedule.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(schedules(&after), schedules(&before));
    assert_eq!(after.queue("main").unwrap().max_active, 1);
    let list = ok(elsewhere.path(), &["queue", "list"]);
    assert!(!list.contains("SECRET-TOKEN"), "{list}");

    // A hostile file: the import fails clearly and changes nothing.
    let queues_before = fs::read(QueueStore::new(elsewhere.path()).path()).unwrap();
    let hostile = home.path().join("hostile.json");
    fs::write(&hostile, "[".repeat(100_000)).unwrap();
    let out = zajil(elsewhere.path(), &["queue", "import", path(&hostile)]);
    assert_eq!(out.status.code(), Some(1), "{}", printed(&out));
    assert!(
        printed(&out).contains("refusing to import"),
        "{}",
        printed(&out)
    );
    assert_eq!(
        fs::read(QueueStore::new(elsewhere.path()).path()).unwrap(),
        queues_before
    );
}
