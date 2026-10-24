//! Export and import of queues (M4, owner's decision on #36 and its
//! success criterion on the M4 plan): an export imported elsewhere gives
//! back exactly the same queues; a damaged or hostile file is refused with
//! a clear error and changes nothing; the exported file holds no user name
//! or password and only its owner can read it.

use std::fs;
use std::path::{Path, PathBuf};

use zajil_core::queue::{
    Day, Folders, QueueStore, QueuesFile, Schedule, Status, TransferError, read_export,
    write_export,
};
use zajil_core::state::checksum_of;

fn dir(name: &str) -> PathBuf {
    std::env::temp_dir().join("zajil-transfer").join(name)
}

/// Two queues with every kind of value an export carries.
fn sample() -> QueuesFile {
    let mut q = QueuesFile::empty();
    q.settings.speed_limit = Some(2 * 1024 * 1024);
    let a = q
        .add("main", "https://example.com/a.iso", &dir("a"))
        .unwrap();
    let b = q
        .add(
            "main",
            "https://cdn.example.net/b.zip?sig=SECRET-TOKEN&e=1",
            &dir("b"),
        )
        .unwrap();
    let c = q
        .add("ليلي", "http://example.org/c.bin", &dir("c"))
        .unwrap();
    let d = q
        .add("ليلي", "http://example.org/d.bin", &dir("d"))
        .unwrap();
    let e = q
        .add("ليلي", "http://example.org/e.bin", &dir("e"))
        .unwrap();
    q.item_mut(a).unwrap().limit = Some(500 * 1024);
    q.item_mut(b).unwrap().categorize = true;
    q.item_mut(b).unwrap().status = Status::Paused;
    q.item_mut(c).unwrap().status = Status::Done;
    q.item_mut(d).unwrap().status = Status::Failed;
    q.item_mut(d).unwrap().error = Some("the server answered 404".into());
    q.item_mut(e).unwrap().status = Status::Active;
    q.item_mut(e).unwrap().state_file = Some(dir("e").join("e.bin.zajil-state"));
    let night = q.queue_mut("ليلي").unwrap();
    night.max_active = 2;
    night.schedule = Some(
        Schedule::new(
            Some("02:00".parse().unwrap()),
            Some("07:00".parse().unwrap()),
            vec![Day::Fri, Day::Sat],
        )
        .unwrap(),
    );
    q
}

/// The queues as the user sees them: what an export must give back.
/// Ids, progress, finished files and errors belong to one computer; a
/// running or failed download is waiting again on the other side.
fn as_seen(q: &QueuesFile) -> serde_json::Value {
    serde_json::json!({
        "speed_limit": q.settings.speed_limit,
        "queues": q.queues.iter().map(|queue| serde_json::json!({
            "name": queue.name,
            "max_active": queue.max_active,
            "schedule": queue.schedule,
            "items": queue.items.iter().map(|i| serde_json::json!({
                "url": i.url,
                "dir": i.dir,
                "status": match i.status {
                    Status::Active | Status::Failed => Status::Waiting,
                    other => other,
                },
                "limit": i.limit,
                "categorize": i.categorize,
                "added_at": i.added_at,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn exported(q: &QueuesFile) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("queues-export.json");
    write_export(&path, &q.export()).unwrap();
    (tmp, path)
}

/// M4 success criterion: export, then import into an empty data folder
/// elsewhere: exactly the same queues, settings and downloads, in order.
#[test]
fn an_export_imported_elsewhere_gives_back_the_same_queues() {
    let original = sample();
    let (_tmp, path) = exported(&original);
    let elsewhere = tempfile::tempdir().unwrap();
    let store = QueueStore::new(elsewhere.path());
    let export = read_export(&path).unwrap();
    store
        .update(|q| {
            // An empty data folder has the empty default queue `main`.
            q.import(&export, &Folders::Keep)
        })
        .unwrap();
    let imported = store.load().unwrap();
    assert_eq!(as_seen(&imported), as_seen(&original));
    // Nothing of this computer went across.
    assert!(
        imported
            .queues
            .iter()
            .flat_map(|q| &q.items)
            .all(|i| { i.state_file.is_none() && i.file.is_none() && i.error.is_none() })
    );
}

#[test]
fn the_export_holds_no_user_name_or_password_and_only_its_owner_can_read_it() {
    let original = sample();
    let (_tmp, path) = exported(&original);
    assert!(zajil_platform::fs::only_owner_can_access(&path).unwrap());
    let text = fs::read_to_string(&path).unwrap();
    for url in text.lines().filter(|l| l.contains("\"url\"")) {
        let url: String =
            serde_json::from_str(url.split_once(':').unwrap().1.trim().trim_end_matches(','))
                .unwrap();
        let parsed = zajil_core::Url::parse(&url).unwrap();
        assert!(
            parsed.username().is_empty() && parsed.password().is_none(),
            "{url}"
        );
    }
    // The token that is part of a URL stays (AGENTS.md §10.3: the file is
    // private, and the token is never printed).
    assert!(text.contains("sig=SECRET-TOKEN"));
}

#[test]
fn an_export_never_writes_over_a_file_or_through_a_link() {
    let tmp = tempfile::tempdir().unwrap();
    let existing = tmp.path().join("there.json");
    fs::write(&existing, b"mine").unwrap();
    assert!(matches!(
        write_export(&existing, &sample().export()),
        Err(TransferError::Exists(_))
    ));
    assert_eq!(fs::read(&existing).unwrap(), b"mine");

    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.txt");
    fs::write(&victim, b"keep me").unwrap();
    let link = tmp.path().join("link.json");
    zajil_platform::fs::symlink_file(&victim, &link)
        .expect("this test must be able to create a symbolic link");
    assert!(matches!(
        write_export(&link, &sample().export()),
        Err(TransferError::Exists(_))
    ));
    assert_eq!(fs::read(&victim).unwrap(), b"keep me");
}

#[test]
fn without_keep_folders_the_downloads_go_to_the_given_folder_by_category() {
    let (_tmp, path) = exported(&sample());
    let export = read_export(&path).unwrap();
    let mut q = QueuesFile::empty();
    let mine = std::env::temp_dir().join("zajil-transfer-mine");
    q.import(&export, &Folders::Into(mine.clone())).unwrap();
    for item in q.queues.iter().flat_map(|q| &q.items) {
        assert_eq!(item.dir, mine);
        assert!(item.categorize);
    }
}

#[test]
fn a_queue_that_exists_keeps_its_settings_and_gets_the_downloads_at_its_end() {
    let (_tmp, path) = exported(&sample());
    let export = read_export(&path).unwrap();
    let mut q = QueuesFile::empty();
    q.settings.speed_limit = Some(1);
    q.add("ليلي", "http://mine.example/x", &dir("x")).unwrap();
    q.queue_mut("ليلي").unwrap().max_active = 7;
    let summary = q.import(&export, &Folders::Keep).unwrap();
    let night = q.queue("ليلي").unwrap();
    assert_eq!(night.max_active, 7);
    assert_eq!(night.items.len(), 4);
    assert_eq!(night.items[0].url, "http://mine.example/x");
    assert_eq!(q.settings.speed_limit, Some(1));
    assert!(!summary.speed_limit_set);
    assert_eq!(summary.items_added, 5);
}

/// Writes `value` with a checksum that matches, so only the checks after
/// the checksum can refuse it.
fn write_signed(path: &Path, mut value: serde_json::Value) {
    value["checksum"] = checksum_of(&value).into();
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

/// M4 success criterion: a damaged or hostile file is refused with a
/// clear message, the program does not crash, and nothing is imported.
#[test]
fn damaged_or_hostile_files_are_refused_clearly_and_change_nothing() {
    let (tmp, path) = exported(&sample());
    let good: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let bad = tmp.path().join("bad.json");
    let refuse = |expect: &str| {
        let err = read_export(&bad).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains(expect),
            "expected {expect:?} in: {message}"
        );
    };

    fs::write(&bad, b"\x00\xff\xfe garbage").unwrap();
    refuse("not valid JSON");
    fs::write(&bad, &fs::read(&path).unwrap()[..200]).unwrap();
    refuse("not valid JSON");
    // Nesting deep enough to overflow a naive recursive parser.
    fs::write(&bad, "[".repeat(100_000)).unwrap();
    refuse("not valid JSON");
    fs::write(&bad, vec![b' '; 16 * 1024 * 1024 + 1]).unwrap();
    refuse("larger than 16 MiB");
    fs::write(&bad, br#"{"format":"zajil-queues","schema_version":1}"#).unwrap();
    refuse("is not a Zajil queues export");

    let mut v = good.clone();
    v["schema_version"] = 2.into();
    write_signed(&bad, v);
    refuse("export version 2");

    // Changed by hand, the checksum left as it was.
    let mut v = good.clone();
    v["queues"][0]["max_active"] = 30.into();
    fs::write(&bad, serde_json::to_vec(&v).unwrap()).unwrap();
    refuse("checksum does not match");

    // Checksum matching, but values no export may hold.
    type Change = Box<dyn Fn(&mut serde_json::Value)>;
    let cases: Vec<(&str, Change)> = vec![
        (
            "user name or password",
            Box::new(|v| {
                v["queues"][0]["items"][0]["url"] = "https://user:pass@example.com/a".into();
            }),
        ),
        (
            "only http and https",
            Box::new(|v| {
                v["queues"][0]["items"][0]["url"] = "file:///etc/passwd".into();
            }),
        ),
        (
            "not absolute or contains `..`",
            Box::new(|v| {
                v["queues"][0]["items"][0]["dir"] = "relative/folder".into();
            }),
        ),
        (
            "not absolute or contains `..`",
            Box::new(|v| {
                let up = std::env::temp_dir().join("..").join("..").join("startup");
                v["queues"][0]["items"][0]["dir"] = up.to_string_lossy().into_owned().into();
            }),
        ),
        (
            "1 to 32 allowed",
            Box::new(|v| v["queues"][0]["max_active"] = 0.into()),
        ),
        (
            "speed limit of 0",
            Box::new(|v| v["queues"][0]["items"][0]["limit"] = 0.into()),
        ),
        (
            "invalid queue name",
            Box::new(|v| v["queues"][0]["name"] = "main\u{202e}gpj".into()),
        ),
        (
            "appears twice",
            Box::new(|v| {
                let first = v["queues"][0].clone();
                v["queues"].as_array_mut().unwrap().push(first);
            }),
        ),
        (
            "unknown field",
            Box::new(|v| v["queues"][0]["items"][0]["run_after"] = "calc.exe".into()),
        ),
        (
            "unknown field",
            Box::new(|v| v["password"] = "hunter2".into()),
        ),
    ];
    for (expect, change) in cases {
        let mut v = good.clone();
        change(&mut v);
        write_signed(&bad, v);
        refuse(expect);
    }

    // A link in place of the file.
    let link = tmp.path().join("link.json");
    zajil_platform::fs::symlink_file(&path, &link)
        .expect("this test must be able to create a symbolic link");
    assert!(matches!(
        read_export(&link),
        Err(TransferError::NotRegularFile(_))
    ));
}
