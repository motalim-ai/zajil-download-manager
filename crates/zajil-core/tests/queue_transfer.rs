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
    // `main` with settings of its own too: a new data folder has a default
    // `main`, which must take them (review of #41).
    let main = q.queue_mut("main").unwrap();
    main.max_active = 1;
    main.schedule = Some(Schedule::new(Some("01:30".parse().unwrap()), None, vec![]).unwrap());
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
    // `main` was touched here (it has a download), so it keeps its own.
    q.add("main", "http://mine.example/y", &dir("y")).unwrap();
    q.queue_mut("ليلي").unwrap().max_active = 7;
    let summary = q.import(&export, &Folders::Keep).unwrap();
    let night = q.queue("ليلي").unwrap();
    assert_eq!(night.max_active, 7);
    assert_eq!(night.items.len(), 4);
    assert_eq!(night.items[0].url, "http://mine.example/x");
    assert_eq!(q.settings.speed_limit, Some(1));
    assert!(!summary.speed_limit_set);
    assert_eq!(summary.items_added, 5);
    assert_eq!(summary.kept_settings, ["main", "ليلي"]);
    assert_eq!(q.queue("main").unwrap().max_active, 3);
    assert!(q.queue("main").unwrap().schedule.is_none());
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

/// Owner's review of #41: no header (`Cookie`, `Authorization` or any
/// other) ever reaches an export. The file holds exactly the documented
/// fields, at every level, and nothing that names a header.
#[test]
fn no_header_ever_goes_into_an_export() {
    let (_tmp, path) = exported(&sample());
    let text = fs::read_to_string(&path).unwrap();
    let lower = text.to_lowercase();
    for word in ["cookie", "authorization", "header", "bearer", "password"] {
        assert!(!lower.contains(word), "{word} in the export:\n{text}");
    }
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let keys = |v: &serde_json::Value| -> Vec<String> {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    assert_eq!(
        keys(&value),
        [
            "checksum",
            "exported_at",
            "format",
            "queues",
            "schema_version",
            "settings"
        ]
    );
    assert_eq!(keys(&value["settings"]), ["speed_limit"]);
    for queue in value["queues"].as_array().unwrap() {
        assert_eq!(keys(queue), ["items", "max_active", "name", "schedule"]);
        for item in queue["items"].as_array().unwrap() {
            assert_eq!(
                keys(item),
                ["added_at", "categorize", "dir", "limit", "status", "url"]
            );
        }
    }
}

/// Review of #41: an import, or any change, that would make the queues
/// file larger than Zajil reads back is refused, and the queues stay
/// readable, the user's own included.
#[test]
fn nothing_ever_writes_a_queues_file_too_large_to_read_back() {
    let (tmp, path) = exported(&sample());
    let good: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let data = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data.path());
    store
        .update(|q| {
            q.add("main", "http://mine.example/mine.iso", &dir("m"))
                .map(|_| ())
        })
        .unwrap();
    let before = fs::read(store.path()).unwrap();

    // 60,000 downloads: the export reads (it is small), but the queues
    // file they would make is too large, so the import is refused whole and
    // nothing changes. No separate count limit: Zajil itself can hold more
    // than such a limit, and it must import its own exports (review of
    // #41, round 2).
    let mut v = good.clone();
    let item = v["queues"][0]["items"][0].clone();
    v["queues"][0]["items"] = serde_json::Value::Array(vec![item; 60_000]);
    let big = tmp.path().join("big.json");
    write_signed(&big, v);
    let export = read_export(&big).unwrap();
    let err = store
        .update(|q| q.import(&export, &Folders::Into(dir("big"))))
        .unwrap_err();
    assert!(
        matches!(err, zajil_core::queue::StoreError::TooLarge { .. }),
        "{err}"
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);

    // Under that number, but long URLs: refused when the queues file would
    // be written, and nothing changes.
    let err = store
        .update(|q| {
            let long = format!("http://example.com/{}", "a".repeat(8 * 1024));
            for _ in 0..2200 {
                q.add("main", &long, &dir("l"))?;
            }
            Ok(())
        })
        .unwrap_err();
    assert!(
        matches!(err, zajil_core::queue::StoreError::TooLarge { .. }),
        "{err}"
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert_eq!(store.load().unwrap().queue("main").unwrap().items.len(), 1);
}

/// Review of #41: schedules, times and URLs in an export are what Zajil
/// itself would write, or the file is refused.
#[test]
fn impossible_schedules_times_and_urls_are_refused() {
    let (tmp, path) = exported(&sample());
    let good: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let bad = tmp.path().join("bad.json");
    type Change = Box<dyn Fn(&mut serde_json::Value)>;
    let cases: Vec<(&str, Change)> = vec![
        (
            "impossible schedule",
            Box::new(|v| {
                v["queues"][1]["schedule"]["days"] =
                    serde_json::Value::Array(vec!["mon".into(); 1_000_000]);
            }),
        ),
        (
            "impossible schedule",
            Box::new(|v| {
                v["queues"][1]["schedule"] =
                    serde_json::json!({"start": null, "stop": null, "days": []});
            }),
        ),
        (
            "impossible schedule",
            Box::new(|v| {
                v["queues"][1]["schedule"]["days"] = serde_json::json!(["sat", "fri"]);
            }),
        ),
        (
            "not an RFC 3339 time",
            Box::new(|v| v["queues"][0]["items"][0]["added_at"] = "yesterday\u{202e}".into()),
        ),
        (
            "not an RFC 3339 time",
            Box::new(|v| v["exported_at"] = "\u{1b}[2J".into()),
        ),
        (
            "control characters or direction marks",
            Box::new(|v| {
                v["queues"][0]["items"][0]["url"] = "\u{1b}http://example.com/a".into();
            }),
        ),
    ];
    for (expect, change) in cases {
        let mut v = good.clone();
        change(&mut v);
        write_signed(&bad, v);
        let err = read_export(&bad).unwrap_err().to_string();
        assert!(err.contains(expect), "expected {expect:?} in: {err}");
    }
}

/// Review of #41: the folders of an export matter only with
/// `--keep-folders`. An export from another system (with folders this one
/// cannot use) imports into a folder of the user's; kept, such folders are
/// refused before anything changes, and so are network and device paths.
#[test]
fn folders_from_the_file_are_checked_only_when_kept() {
    let (tmp, path) = exported(&sample());
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    // A folder that is not absolute on this system, whichever it is.
    let foreign = if cfg!(windows) {
        "/home/me/Downloads"
    } else {
        "C:\\Users\\me\\Downloads"
    };
    v["queues"][0]["items"][0]["dir"] = foreign.into();
    let other = tmp.path().join("other-system.json");
    write_signed(&other, v);
    let export = read_export(&other).unwrap();

    let mut q = QueuesFile::empty();
    let mine = std::env::temp_dir().join("zajil-transfer-mine");
    q.import(&export, &Folders::Into(mine.clone())).unwrap();
    assert!(
        q.queues
            .iter()
            .flat_map(|q| &q.items)
            .all(|i| i.dir == mine)
    );

    let mut q = QueuesFile::empty();
    let before = q.clone();
    let err = q.import(&export, &Folders::Keep).unwrap_err().to_string();
    assert!(err.contains("not a plain absolute path"), "{err}");
    assert_eq!(q, before, "a refused import changed something");

    // Control and invisible characters (review of #41, round 2: escape
    // codes reached the terminal through `zajil queue run`).
    let escape = std::env::temp_dir().join("x\u{1b}]0;PWNED\u{7}\u{1b}[31m");
    let spoof = std::env::temp_dir().join("photos\u{202e}gpj.exe");
    for path in [
        "\\\\server\\share\\x",
        "\\\\?\\C:\\x",
        "relative/x",
        "/tmp/../etc",
        escape.to_str().unwrap(),
        spoof.to_str().unwrap(),
    ] {
        let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&other).unwrap()).unwrap();
        v["queues"][0]["items"][0]["dir"] = path.into();
        write_signed(&other, v);
        let export = read_export(&other).unwrap();
        let err = QueuesFile::empty()
            .import(&export, &Folders::Keep)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("not a plain absolute path") || err.contains("control or invisible"),
            "{path:?}: {err}"
        );
    }
}

/// Review of #41: an export never removes or writes over anything: not a
/// `<file>.tmp` of the user's, and leaves no temporary file behind.
#[test]
fn an_export_removes_nothing_and_leaves_no_temporary_file() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("out.json");
    let users = tmp.path().join("out.json.tmp");
    fs::write(&users, b"my notes").unwrap();
    write_export(&target, &sample().export()).unwrap();
    assert_eq!(fs::read(&users).unwrap(), b"my notes");
    let mut names: Vec<String> = fs::read_dir(tmp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["out.json", "out.json.tmp"]);
    assert!(zajil_platform::fs::only_owner_can_access(&target).unwrap());
    read_export(&target).unwrap();
}

/// Many queues import in linear time: each queue was looked for by a
/// search through all of them, so 360,000 queues (a file under the size
/// limit) took 88 s while holding the queues lock (review of #41, round 3).
/// Quadratic, these 200,000 would take minutes even in a release build.
#[test]
fn many_queues_import_without_a_search_per_queue() {
    let mut q = QueuesFile::empty();
    for n in 0..200_000 {
        q.queues.push(zajil_core::queue::Queue {
            name: format!("q{n}"),
            max_active: 3,
            schedule: None,
            items: Vec::new(),
        });
    }
    q.add("q199999", "https://example.com/last.iso", &dir("last"))
        .unwrap();
    let export = q.export();
    let mut into = QueuesFile::empty();
    let started = std::time::Instant::now();
    let summary = into.import(&export, &Folders::Keep).unwrap();
    let took = started.elapsed();
    assert_eq!(summary.queues_created, 200_000);
    assert_eq!(into.queue("q199999").unwrap().items.len(), 1);
    assert!(took < std::time::Duration::from_secs(20), "{took:?}");
}

/// The longest file name a folder can hold (255 bytes) still takes an
/// export: the temporary file has a short name of its own (review of #41,
/// rounds 2 and 3). Unix only: on Windows the whole path, not the name,
/// would pass the 260-character limit first.
#[cfg(unix)]
#[test]
fn an_export_to_a_255_byte_name_works() {
    let tmp = tempfile::tempdir().unwrap();
    let ascii = tmp.path().join(format!("{}.json", "a".repeat(250)));
    write_export(&ascii, &sample().export()).unwrap();
    read_export(&ascii).unwrap();
    // 2 bytes a letter: 125 letters and `.json` are 255 bytes.
    let arabic = tmp.path().join(format!("{}.json", "ص".repeat(125)));
    write_export(&arabic, &sample().export()).unwrap();
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 2);
}

/// Invisible characters that could disguise a name are refused in kept
/// folders and in category names and folders (review of #41, round 3):
/// Hangul fillers, the combining grapheme joiner, direction marks. Joiners
/// and variation selectors are part of real text and stay allowed (round
/// 4): emoji, Persian with its ZWNJ, an ideographic variant, Sinhala.
#[test]
fn invisible_characters_are_refused_but_joiners_and_variation_selectors_are_not() {
    use zajil_core::category::{check_folder, check_name};
    let import_keeping = |folder: &Path| {
        let mut q = QueuesFile::empty();
        q.add("main", "https://example.com/a.iso", folder).unwrap();
        let (_tmp, path) = exported(&q);
        let export = read_export(&path).unwrap();
        let mut into = QueuesFile::empty();
        into.import(&export, &Folders::Keep).map(|_| into)
    };
    for c in [
        '\u{3164}', '\u{FFA0}', '\u{115F}', '\u{034F}', '\u{17B4}', '\u{202E}', '\u{200E}',
        '\u{2066}',
    ] {
        let folder = std::env::temp_dir().join(format!("a{c}b"));
        assert!(check_folder(&folder).is_err(), "{:04X}", c as u32);
        assert!(check_name(&format!("a{c}b")).is_err(), "{:04X}", c as u32);
        assert!(import_keeping(&folder).is_err(), "{:04X}", c as u32);
        // Queue names too: the Hangul fillers count as letters.
        assert!(
            QueuesFile::empty()
                .add(&format!("ليلي{c}"), "https://example.com/a", &folder)
                .is_err()
        );
    }
    for text in [
        "❤️ صور",
        "1️⃣",
        "فیلم‌ها",
        "葛󠄀飾",
        "ශ්‍රී",
        "\u{F8FF} Apple",
        "مُستنداتٌ العمل",
    ] {
        let folder = std::env::temp_dir().join(text);
        assert!(check_folder(&folder).is_ok(), "{text}");
        assert!(check_name(text).is_ok(), "{text}");
        assert!(import_keeping(&folder).is_ok(), "{text}");
    }
    // A name of nothing but variation selectors shows nothing.
    assert!(check_name("\u{FE0F}\u{FE0F}").is_err());
}

/// Zajil imports its own exports whatever the URLs hold: an emoji with its
/// variation selector or a Persian ZWNJ was refused (review of #41, round
/// 4). What would drive a terminal or reorder text is refused when added,
/// so it never reaches an export.
#[test]
fn urls_that_can_be_added_can_be_imported_back() {
    let mut q = QueuesFile::empty();
    for url in [
        "https://example.com/❤️.png",
        "https://fa.wikipedia.org/wiki/فیلم‌ها",
        "https://example.jp/葛󠄀飾.pdf",
    ] {
        q.add("main", url, &dir("u")).unwrap();
    }
    let (_tmp, path) = exported(&q);
    let export = read_export(&path).unwrap();
    let mut into = QueuesFile::empty();
    into.import(&export, &Folders::Into(dir("v"))).unwrap();
    let urls = |q: &QueuesFile| {
        q.queues[0]
            .items
            .iter()
            .map(|i| i.url.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(urls(&into), urls(&q));
    for bad in [
        "https://example.com/a\u{202E}gpj.exe",
        "https://example.com/\u{1b}[31m",
        "https://example.com/a\u{2029}b",
    ] {
        assert!(
            QueuesFile::empty().add("main", bad, &dir("u")).is_err(),
            "{bad:?}"
        );
    }
}
