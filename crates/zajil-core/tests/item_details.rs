//! What the user gives a queued download (M5.5, #95 and #103): a file name
//! of its own, a category, a description; and the time of its last try,
//! which the runner writes. Against the local test server.

use std::fs;
use std::path::Path;
use std::time::Duration;

use zajil_core::category::CategoryStore;
use zajil_core::queue::{
    MAX_DESCRIPTION_CHARS, QueueEvent, QueueStore, Runner, Status, StoreError, read_export,
    write_export,
};
use zajil_core::sanitize::MAX_NAME_BYTES;
use zajil_core::state::checksum_of;
use zajil_core::{Engine, Options, Stop};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

fn runner(store: &QueueStore) -> Runner {
    let engine = Engine::new(Options::default()).unwrap();
    Runner::new(engine, store.clone()).poll(Duration::from_millis(50))
}

fn add(store: &QueueStore, url: &str, dir: &Path) -> u64 {
    store.update(|q| q.add("main", url, dir)).unwrap()
}

fn set_status(store: &QueueStore, id: u64, status: Status) {
    store
        .update(|q| {
            q.item_mut(id).unwrap().status = status;
            Ok(())
        })
        .unwrap();
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// A server that names its file `server.zip`.
fn server_naming(path: &str, name: &str, data: &[u8]) -> TestServer {
    let server = TestServer::http();
    server.put(
        path,
        FileSpec::new(data.to_vec())
            .content_disposition(&format!("attachment; filename=\"{name}\"")),
    );
    server
}

/// #103: the file is saved under the user's name, cleaned like any name
/// (no folder in it), not the server's.
#[tokio::test]
async fn a_download_is_saved_under_the_name_the_user_gave_it() {
    let data = random_bytes(200_000, 950);
    let server = server_naming("/get", "server.zip", &data);
    let data_dir = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let id = add(&store, &server.url("/get"), dir.path());
    let given = store
        .update(|q| q.set_name(id, Some("../../تقرير الربع.zip")))
        .unwrap();
    assert_eq!(given.as_deref(), Some("تقرير الربع.zip"));

    let summary = runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    assert_eq!(summary.finished, 1, "{summary:?}");
    assert_eq!(names_in(dir.path()), ["تقرير الربع.zip"]);
    let file = dir.path().join("تقرير الربع.zip");
    assert_eq!(sha256_hex(&fs::read(&file).unwrap()), sha256_hex(&data));
    let item = store.load().unwrap().item(id).unwrap().clone();
    assert_eq!(item.name.as_deref(), Some("تقرير الربع.zip"));

    // No name, or an empty one: the server's name again.
    let id = add(&store, &server.url("/get"), dir.path());
    store.update(|q| q.set_name(id, Some("a.zip"))).unwrap();
    assert_eq!(store.update(|q| q.set_name(id, Some("  "))).unwrap(), None);
    runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    assert!(dir.path().join("server.zip").is_file());
}

/// AGENTS.md §10.7: a long name of multi-byte characters, taken already,
/// is numbered within the limit, and the file it gets is downloaded.
#[tokio::test]
async fn a_long_taken_name_is_numbered_within_the_limit() {
    let data = random_bytes(50_000, 951);
    let server = server_naming("/get", "server.bin", &data);
    let data_dir = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    for total in [197, 198, 199, 200] {
        // `ع` is 2 bytes; `.bin` 4.
        let stem = "ع".repeat((total - 4) / 2);
        let wanted = format!("{stem}{}.bin", "x".repeat((total - 4) % 2));
        assert_eq!(wanted.len(), total);
        fs::write(dir.path().join(&wanted), b"the user's own file").unwrap();
        let id = add(&store, &server.url("/get"), dir.path());
        let kept = store.update(|q| q.set_name(id, Some(&wanted))).unwrap();
        assert_eq!(kept.as_deref(), Some(wanted.as_str()), "{total}");
        runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
        let item = store.load().unwrap().item(id).unwrap().clone();
        assert_eq!(item.status, Status::Done, "{total}: {item:?}");
        let file = item.file.unwrap();
        let got = file.file_name().unwrap().to_str().unwrap();
        assert!(got.len() <= MAX_NAME_BYTES, "{got}");
        // The user's name, numbered: not the server's.
        // (Shortened to make room, earlier rounds' names may share its
        // stem: any number.)
        let numbered = got
            .strip_suffix(").bin")
            .and_then(|rest| rest.rsplit_once(" ("))
            .is_some_and(|(_, n)| n.parse::<u32>().is_ok());
        assert!(got.starts_with('ع') && numbered, "{got}");
        assert_eq!(sha256_hex(&fs::read(&file).unwrap()), sha256_hex(&data));
        assert_eq!(
            fs::read(dir.path().join(&wanted)).unwrap(),
            b"the user's own file"
        );
    }
}

/// AGENTS.md §10.1: a link with the user's name, to a file outside the
/// folder, is never written through.
#[tokio::test]
async fn a_link_with_the_user_s_name_is_never_written_through() {
    let data = random_bytes(50_000, 952);
    let server = server_naming("/get", "server.bin", &data);
    let data_dir = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    for (n, kind) in ["symbolic", "hard"].into_iter().enumerate() {
        let victim = elsewhere.path().join(format!("victim-{n}.txt"));
        fs::write(&victim, b"not yours").unwrap();
        let name = format!("mine-{n}.bin");
        let link = dir.path().join(&name);
        if kind == "symbolic" {
            zajil_platform::fs::symlink_file(&victim, &link)
                .expect("symbolic links must work here");
        } else {
            fs::hard_link(&victim, &link).unwrap();
        }
        let id = add(&store, &server.url("/get"), dir.path());
        store.update(|q| q.set_name(id, Some(&name))).unwrap();
        runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
        // Refused, as a link with the server's name is; never followed.
        let item = store.load().unwrap().item(id).unwrap().clone();
        match item.status {
            Status::Failed => assert!(
                item.error.as_deref().is_some_and(|e| e.contains("link")),
                "{kind}: {item:?}"
            ),
            Status::Done => {
                let file = item.file.unwrap();
                assert_eq!(
                    dunce::canonicalize(file.parent().unwrap()).unwrap(),
                    dunce::canonicalize(dir.path()).unwrap()
                );
                assert_ne!(
                    file.file_name().unwrap(),
                    link.file_name().unwrap(),
                    "{kind}"
                );
            }
            _ => panic!("{kind}: {item:?}"),
        }
        assert_eq!(fs::read(&victim).unwrap(), b"not yours", "{kind}");
    }
}

/// #103 with #90: a name that can run code where the server's could not
/// is held for the user before anything is made, and goes once the user
/// starts it.
#[tokio::test]
async fn a_name_that_runs_code_where_the_server_s_did_not_is_held() {
    let data = random_bytes(50_000, 953);
    let server = server_naming("/get", "report.pdf", &data);
    let data_dir = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let id = add(&store, &server.url("/get"), dir.path());
    store
        .update(|q| q.set_name(id, Some("report.exe")))
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
    assert!(names_in(dir.path()).is_empty(), "a file was made");

    // The user starts it: it goes, and is not held again.
    set_status(&store, id, Status::Waiting);
    runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    let item = store.load().unwrap().item(id).unwrap().clone();
    assert_eq!(item.status, Status::Done, "{item:?}");
    assert_eq!(names_in(dir.path()), ["report.exe"]);
}

/// A new name asks again: the user's OK was for the name it had.
#[tokio::test]
async fn a_new_name_takes_back_the_user_s_ok() {
    let server = server_naming("/get", "report.pdf", &random_bytes(20_000, 957));
    let data_dir = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let id = add(&store, &server.url("/get"), dir.path());
    store
        .update(|q| {
            q.item_mut(id).unwrap().ok_given = true;
            q.set_name(id, Some("report.exe"))
        })
        .unwrap();
    assert!(!store.load().unwrap().item(id).unwrap().ok_given);
    runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    let item = store.load().unwrap().item(id).unwrap().clone();
    assert_eq!(
        (item.status, item.needs_ok),
        (Status::Paused, true),
        "{item:?}"
    );
    assert!(names_in(dir.path()).is_empty());
}

/// A name with nothing in it that can be a file name is refused, not
/// turned into `download`.
#[test]
fn a_name_with_nothing_of_a_name_is_refused() {
    let data_dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let id = add(&store, "http://h/a", Path::new("/"));
    for bad in [".", "..", "\u{202E}", "/", "\\..\\"] {
        let refused = store.update(|q| q.set_name(id, Some(bad)));
        assert!(matches!(refused, Err(StoreError::Refused(_))), "{bad:?}");
    }
    // Asked for by name, it is a name.
    assert_eq!(
        store
            .update(|q| q.set_name(id, Some("download")))
            .unwrap()
            .as_deref(),
        Some("download")
    );
}

/// Not held: renamed, but the server's name ran code already (nothing
/// changes for the user); and from a page, renamed to a name that cannot
/// run code (the file it makes cannot).
#[tokio::test]
async fn renaming_is_held_only_when_the_name_brings_code() {
    let data = random_bytes(50_000, 954);
    let server = server_naming("/get", "setup.exe", &data);
    let data_dir = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let renamed = add(&store, &server.url("/get"), dir.path());
    store
        .update(|q| q.set_name(renamed, Some("installer.msi")))
        .unwrap();
    let from_page = add(&store, &server.url("/get"), dir.path());
    store
        .update(|q| {
            q.item_mut(from_page).unwrap().from_page = true;
            q.set_name(from_page, Some("setup.txt"))
        })
        .unwrap();
    runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    let queues = store.load().unwrap();
    for id in [renamed, from_page] {
        let item = queues.item(id).unwrap();
        assert_eq!(
            (item.status, item.needs_ok),
            (Status::Done, false),
            "{item:?}"
        );
    }
    assert_eq!(names_in(dir.path()), ["installer.msi", "setup.txt"]);

    // From a page and renamed to a name that runs code: held, as #90.
    let held = add(&store, &server.url("/get"), dir.path());
    store
        .update(|q| {
            q.item_mut(held).unwrap().from_page = true;
            q.set_name(held, Some("other.exe"))
        })
        .unwrap();
    runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    let item = store.load().unwrap().item(held).unwrap().clone();
    assert_eq!((item.status, item.needs_ok), (Status::Paused, true));
}

/// The name is the file's once the download has started: a change then
/// is refused, and nothing changes.
#[test]
fn the_name_cannot_change_once_the_download_has_started() {
    let data_dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let started = add(&store, "http://h/a", Path::new("/"));
    let active = add(&store, "http://h/b", Path::new("/"));
    let done = add(&store, "http://h/c", Path::new("/"));
    store
        .update(|q| {
            let item = q.item_mut(started).unwrap();
            item.status = Status::Paused;
            item.state_file = Some("/x/a.zajil-state".into());
            q.item_mut(active).unwrap().status = Status::Active;
            q.item_mut(done).unwrap().status = Status::Done;
            Ok(())
        })
        .unwrap();
    let before = store.load().unwrap();
    for id in [started, active, done] {
        let refused = store.update(|q| q.set_name(id, Some("new.bin")));
        assert!(
            matches!(refused, Err(StoreError::Refused(_))),
            "{refused:?}"
        );
    }
    assert!(matches!(
        store.update(|q| q.set_name(999, Some("new.bin"))),
        Err(StoreError::NoSuchItem(999))
    ));
    assert_eq!(store.load().unwrap().queues, before.queues);
}

/// #103: for a download added without a folder of its own, the user's
/// category wins over its name and its site; one removed since counts as
/// none, and one without a folder leaves it in the given folder. With a
/// folder the user chose, the category is a label: the folder stays.
#[tokio::test]
async fn a_chosen_category_s_folder_wins_and_a_removed_one_counts_as_none() {
    let data = random_bytes(30_000, 955);
    let server = server_naming("/get", "photos.zip", &data);
    let home = tempfile::tempdir().unwrap();
    let given = home.path().join("downloads");
    let archives = home.path().join("archives");
    let from_site = home.path().join("from-the-test-server");
    let books = home.path().join("كتب");
    fs::create_dir_all(&given).unwrap();
    let categories = CategoryStore::new(home.path());
    categories
        .update(|file| {
            file.get_mut("archives")?.folder = Some(archives.clone());
            file.add("local", "Local")?;
            let local = file.get_mut("local")?;
            local.sites.push("127.0.0.1".into());
            local.folder = Some(from_site.clone());
            file.add("books", "كتب")?;
            file.get_mut("books")?.folder = Some(books.clone());
            Ok(())
        })
        .unwrap();
    let store = QueueStore::new(home.path().join("data"));
    let by_category = |category: &str| {
        let id = add(&store, &server.url("/get"), &given);
        store
            .update(|q| {
                q.item_mut(id).unwrap().categorize = true;
                q.set_category(id, Some(category))
            })
            .unwrap();
        id
    };
    let chosen = by_category("books");
    let removed = by_category("gone");
    let without_folder = by_category("video");
    let own_folder = add(&store, &server.url("/get"), &given);
    store
        .update(|q| q.set_category(own_folder, Some("books")))
        .unwrap();
    runner(&store)
        .categories(categories)
        .run(&Stop::new(), &mut |_| {})
        .await
        .unwrap();
    let queues = store.load().unwrap();
    let folder_of = |id| {
        let item = queues.item(id).unwrap();
        assert_eq!(item.status, Status::Done, "{item:?}");
        let file = item.file.clone().unwrap();
        assert_eq!(sha256_hex(&fs::read(&file).unwrap()), sha256_hex(&data));
        dunce::canonicalize(file.parent().unwrap()).unwrap()
    };
    let canon = |p: &Path| dunce::canonicalize(p).unwrap();
    // Over the site rule (`local`) and the extension (`archives`).
    assert_eq!(folder_of(chosen), canon(&books));
    // Removed: the site rule, as without a choice.
    assert_eq!(folder_of(removed), canon(&from_site));
    assert_eq!(folder_of(without_folder), canon(&given));
    assert_eq!(folder_of(own_folder), canon(&given));
    // The category stays on the item, for the interface.
    assert_eq!(
        queues.item(own_folder).unwrap().category.as_deref(),
        Some("books")
    );
}

/// A category is named by its id only.
#[test]
fn a_category_is_set_by_its_id() {
    let data_dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let id = add(&store, "http://h/a", Path::new("/"));
    for bad in ["Books", "../x", "", "a b", "كتب"] {
        let refused = store.update(|q| q.set_category(id, Some(bad)));
        assert!(matches!(refused, Err(StoreError::Refused(_))), "{bad:?}");
    }
    store
        .update(|q| q.set_category(id, Some("my-books_2")))
        .unwrap();
    store.update(|q| q.set_category(id, None)).unwrap();
    assert_eq!(store.load().unwrap().item(id).unwrap().category, None);
    // A finished download can be put in a category: only its label.
    set_status(&store, id, Status::Done);
    store.update(|q| q.set_category(id, Some("video"))).unwrap();
}

/// #95: a description is lines of plain text, at most 1000 characters.
#[test]
fn descriptions_are_lines_of_plain_text() {
    let data_dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let id = add(&store, "http://h/a", Path::new("/"));
    let good = [
        "نسخة العمل، الإصدار 2".to_owned(),
        "سطر أول\nSecond line\n\nفیلم‌ها ❤️".to_owned(),
        "ع".repeat(MAX_DESCRIPTION_CHARS),
    ];
    for text in &good {
        store.update(|q| q.set_description(id, Some(text))).unwrap();
        let read = store.load().unwrap().item(id).unwrap().description.clone();
        assert_eq!(read.as_ref(), Some(text));
    }
    let too_long = "a".repeat(MAX_DESCRIPTION_CHARS + 1);
    for bad in [
        too_long.as_str(),
        "setup\u{202E}fdp.exe",
        "a\r\nb",
        "a\u{1b}[31mb",
        "Vi\u{200C}deos",
        "a\u{3164}b",
    ] {
        let refused = store.update(|q| q.set_description(id, Some(bad)));
        assert!(matches!(refused, Err(StoreError::Refused(_))), "{bad:?}");
    }
    store
        .update(|q| q.set_description(id, Some(" \n ")))
        .unwrap();
    assert_eq!(store.load().unwrap().item(id).unwrap().description, None);
}

/// #95: the runner writes when it last started an item. An item without
/// the new details is written without them (until its first run, the file
/// is as before).
#[tokio::test]
async fn the_runner_writes_the_time_of_the_last_try() {
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(random_bytes(20_000, 956)));
    let data_dir = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let id = add(&store, &server.url("/f.bin"), dir.path());
    let json: serde_json::Value = serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
    let written = json["queues"][0]["items"][0].as_object().unwrap();
    for field in ["name", "category", "description", "last_try"] {
        assert!(!written.contains_key(field), "{field} in {written:?}");
    }
    let before = jiff::Timestamp::now();
    runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    let item = store.load().unwrap().item(id).unwrap().clone();
    let last_try: jiff::Timestamp = item.last_try.as_deref().unwrap().parse().unwrap();
    // RFC 3339 keeps whole seconds or more: a margin of one.
    assert!(last_try >= before - jiff::SignedDuration::from_secs(1));
    assert!(last_try <= jiff::Timestamp::now());
    assert!(item.last_try.unwrap().ends_with('Z'));
}

/// The queues file is read strictly: details its setters would refuse are
/// a damaged file, even with a matching checksum.
#[test]
fn a_queues_file_with_details_the_setters_refuse_is_damaged() {
    let bad: [(&str, serde_json::Value, &str); 6] = [
        ("name", "../evil.exe".into(), "file name"),
        ("name", "a\u{7}b".into(), "file name"),
        ("category", "Not An Id".into(), "category"),
        ("description", "a\u{202E}b".into(), "description"),
        ("last_try", "yesterday".into(), "last_try"),
        ("last_try", "2026-10-25T10:00:00+02:00".into(), "last_try"),
    ];
    for (field, value, reason) in bad {
        let data_dir = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data_dir.path());
        add(&store, "http://h/a", Path::new("/"));
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
        json["queues"][0]["items"][0][field] = value;
        json["checksum"] = "".into();
        json["checksum"] = checksum_of(&json).into();
        fs::write(store.path(), serde_json::to_vec(&json).unwrap()).unwrap();
        let err = store.load().unwrap_err();
        assert!(matches!(err, StoreError::Corrupt { .. }), "{field}: {err}");
        assert!(err.to_string().contains(reason), "{field}: {err}");
    }
}

/// The user's details go with an export and come back with the import;
/// the time of the last try does not (it is this computer's). An export
/// with details the setters refuse is refused whole.
#[test]
fn the_user_s_details_go_with_an_export_and_the_last_try_does_not() {
    let data_dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let id = add(&store, "http://h/a", Path::new("/"));
    store
        .update(|q| {
            q.set_name(id, Some("تقرير.pdf"))?;
            q.set_category(id, Some("documents"))?;
            q.set_description(id, Some("للاجتماع\nيوم الأحد"))?;
            q.item_mut(id).unwrap().last_try = Some("2026-10-25T10:00:00Z".into());
            Ok(())
        })
        .unwrap();
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("queues-export.json");
    write_export(&path, &store.load().unwrap().export()).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains("last_try") && !text.contains("2026-10-25T10"),
        "{text}"
    );

    let other = tempfile::tempdir().unwrap();
    let other = QueueStore::new(other.path());
    let export = read_export(&path).unwrap();
    other
        .update(|q| q.import(&export, &zajil_core::queue::Folders::Keep))
        .unwrap();
    let imported = other.load().unwrap();
    let item = &imported.queue("main").unwrap().items[0];
    assert_eq!(item.name.as_deref(), Some("تقرير.pdf"));
    assert_eq!(item.category.as_deref(), Some("documents"));
    assert_eq!(item.description.as_deref(), Some("للاجتماع\nيوم الأحد"));
    assert_eq!(item.last_try, None);

    for (field, value, reason) in [
        ("name", "..\\evil.exe", "file name"),
        ("category", "../x", "category"),
        ("description", "a\u{202E}b", "description"),
    ] {
        let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
        json["queues"][0]["items"][0][field] = value.into();
        json["checksum"] = "".into();
        json["checksum"] = checksum_of(&json).into();
        let hostile = out.path().join(format!("hostile-{field}.json"));
        fs::write(&hostile, serde_json::to_vec(&json).unwrap()).unwrap();
        let err = read_export(&hostile).unwrap_err();
        let err = err.to_string();
        assert!(
            err.contains("refusing to import") && err.contains(reason),
            "{field}: {err}"
        );
    }
}

/// AGENTS.md §10.7: the engine accepts what it wrote. A download saved
/// under a long user name, numbered, paused in the middle, resumes from
/// its state file and finishes whole.
#[tokio::test]
async fn a_long_numbered_user_name_resumes() {
    let data = random_bytes(4 * 1024 * 1024, 958);
    let server = TestServer::http();
    server.put(
        "/get",
        FileSpec::new(data.clone())
            .content_disposition("attachment; filename=\"server.bin\"")
            .throttle(2 * 1024 * 1024),
    );
    let data_dir = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let wanted = format!("{}.bin", "ع".repeat(98));
    assert_eq!(wanted.len(), 200);
    fs::write(dir.path().join(&wanted), b"the user's own file").unwrap();
    let id = add(&store, &server.url("/get"), dir.path());
    store.update(|q| q.set_name(id, Some(&wanted))).unwrap();

    let steering = store.clone();
    let mut paused = false;
    runner(&store)
        .run(&Stop::new(), &mut |event| {
            if let QueueEvent::Download {
                event: zajil_core::Event::Progress { done, .. },
                ..
            } = event
                && done > 1024 * 1024
                && !paused
            {
                paused = true;
                steering
                    .update(|q| {
                        q.item_mut(id).unwrap().status = Status::Paused;
                        Ok(())
                    })
                    .unwrap();
            }
        })
        .await
        .unwrap();
    assert!(paused);
    let item = store.load().unwrap().item(id).unwrap().clone();
    assert_eq!(item.status, Status::Paused, "{item:?}");
    let state_file = item.state_file.expect("no state file");
    let state_name = state_file.file_name().unwrap().to_str().unwrap();
    // The name is within the limit; its suffix goes on top, within the
    // file systems' 255 bytes.
    assert!(
        state_name.len() <= MAX_NAME_BYTES + ".zajil-state".len(),
        "{state_name}"
    );

    set_status(&store, id, Status::Waiting);
    runner(&store).run(&Stop::new(), &mut |_| {}).await.unwrap();
    let item = store.load().unwrap().item(id).unwrap().clone();
    assert_eq!(item.status, Status::Done, "{item:?}");
    let file = item.file.unwrap();
    let got = file.file_name().unwrap().to_str().unwrap();
    assert!(got.starts_with('ع') && got.ends_with(" (1).bin"), "{got}");
    assert_eq!(sha256_hex(&fs::read(&file).unwrap()), sha256_hex(&data));
    // Resumed, not downloaded again from the start.
    assert!(
        server.gets("/get").iter().any(|g| g.range.is_some()),
        "it started over"
    );
    assert_eq!(
        fs::read(dir.path().join(&wanted)).unwrap(),
        b"the user's own file"
    );
}
