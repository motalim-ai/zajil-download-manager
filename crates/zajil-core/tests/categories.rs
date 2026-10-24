//! Categories (M4): the file `categories.json`, and downloads that go to
//! their category's folder. The engine tests run against the local test
//! server.

use std::fs;
use std::path::Path;

use zajil_core::category::{CategoriesFile, CategoryError, CategoryStore};
use zajil_core::state::checksum_of;
use zajil_core::{Engine, Options, Stop};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

/// The same file, whatever links the temporary folder's path goes through
/// (macOS `/var` is `/private/var`).
fn assert_same_file(a: &Path, b: &Path) {
    assert_eq!(
        dunce::canonicalize(a).unwrap(),
        dunce::canonicalize(b).unwrap()
    );
}

fn store_with_archives_in(data: &Path, archives: &Path) -> CategoryStore {
    let store = CategoryStore::new(data);
    store
        .update(|file| {
            file.get_mut("archives")?.folder = Some(archives.to_owned());
            Ok(())
        })
        .unwrap();
    store
}

/// What a damaged or hostile categories file must never do: be used, or
/// be written over.
fn assert_refused_and_kept(store: &CategoryStore, expect: &str) {
    let before = fs::read(store.path()).ok();
    let err = store.load().unwrap_err();
    assert!(err.to_string().contains(expect), "{err}");
    let change = store.update(|file| file.add("x", "X"));
    assert!(change.is_err());
    assert_eq!(fs::read(store.path()).ok(), before, "the file was changed");
}

#[test]
fn without_a_file_there_are_the_five_built_in_categories() {
    let data = tempfile::tempdir().unwrap();
    let file = CategoryStore::new(data.path()).load().unwrap();
    let ids: Vec<_> = file.categories.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["programs", "video", "music", "documents", "archives"]);
    assert!(file.categories.iter().all(|c| c.folder.is_none()));
    assert!(!CategoryStore::new(data.path()).path().exists());
}

#[test]
fn a_change_is_kept_in_a_file_only_its_owner_can_read() {
    let data = tempfile::tempdir().unwrap();
    let archives = data.path().join("archives");
    let store = store_with_archives_in(data.path(), &archives);
    store
        .update(|file| {
            file.add("comics", "قصص مصورة")?;
            file.get_mut("comics")?.extensions.push("cbz".into());
            Ok(())
        })
        .unwrap();
    // A new store reads it from disk, as a new process would.
    let again = CategoryStore::new(data.path()).load().unwrap();
    assert_eq!(
        again.get("archives").unwrap().folder.as_deref(),
        Some(archives.as_path())
    );
    assert_eq!(again.classify("a.cbz", None).unwrap().id, "comics");
    assert_eq!(
        again.get("comics").unwrap().name.as_deref(),
        Some("قصص مصورة")
    );
    assert!(zajil_platform::fs::only_owner_can_access(&store.path()).unwrap());
    let text = fs::read_to_string(store.path()).unwrap();
    assert!(text.contains("\"format\": \"zajil-categories\""), "{text}");
    assert!(text.contains("\"schema_version\": 1"), "{text}");
}

#[test]
fn a_damaged_file_is_refused_and_never_written_over() {
    let data = tempfile::tempdir().unwrap();
    let store = store_with_archives_in(data.path(), &data.path().join("a"));
    let good = fs::read_to_string(store.path()).unwrap();

    // A value changed by hand, the checksum left as it was.
    fs::write(store.path(), good.replace("\"programs\"", "\"programz\"")).unwrap();
    assert_refused_and_kept(&store, "checksum does not match");

    fs::write(store.path(), &good[..good.len() / 2]).unwrap();
    assert_refused_and_kept(&store, "damaged");

    // An unknown field, with a checksum that matches: still refused.
    let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
    value["categories"][0]["run"] = "rm -rf /".into();
    value["checksum"] = checksum_of(&value).into();
    fs::write(store.path(), value.to_string()).unwrap();
    assert_refused_and_kept(&store, "unknown field");

    // An invisible direction mark in a name, checksum matching.
    let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
    value["categories"][0]["name"] = "exe\u{202e}fdp.zip".into();
    value["checksum"] = checksum_of(&value).into();
    fs::write(store.path(), value.to_string()).unwrap();
    assert_refused_and_kept(&store, "invalid category name");

    // A relative folder, checksum matching.
    let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
    value["categories"][4]["folder"] = "../../etc".into();
    value["checksum"] = checksum_of(&value).into();
    fs::write(store.path(), value.to_string()).unwrap();
    assert_refused_and_kept(&store, "invalid folder");

    let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
    value["schema_version"] = 99.into();
    fs::write(store.path(), value.to_string()).unwrap();
    assert_refused_and_kept(&store, "schema version 99");

    fs::write(store.path(), vec![b' '; 1024 * 1024 + 1]).unwrap();
    assert_refused_and_kept(&store, "too large");
}

#[test]
fn a_link_in_place_of_the_file_is_refused_and_what_it_points_to_is_untouched() {
    let data = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    // A real categories file elsewhere, so a link to it would parse.
    let elsewhere = store_with_archives_in(outside.path(), &outside.path().join("a"));
    let target_before = fs::read(elsewhere.path()).unwrap();
    let store = CategoryStore::new(data.path());

    zajil_platform::fs::symlink_file(&elsewhere.path(), &store.path())
        .expect("this test must be able to create a symbolic link");
    assert!(matches!(
        store.load(),
        Err(CategoryError::NotRegularFile(_))
    ));
    assert!(store.update(|file| file.add("x", "X")).is_err());
    assert_eq!(fs::read(elsewhere.path()).unwrap(), target_before);
    fs::remove_file(store.path()).unwrap();

    fs::hard_link(elsewhere.path(), store.path()).unwrap();
    assert!(matches!(
        store.load(),
        Err(CategoryError::NotRegularFile(_))
    ));
    assert!(store.update(|file| file.add("x", "X")).is_err());
    assert_eq!(fs::read(elsewhere.path()).unwrap(), target_before);
}

#[test]
fn the_lock_file_is_never_followed_through_a_link() {
    let data = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.txt");
    fs::write(&victim, b"keep me").unwrap();
    zajil_platform::fs::symlink_file(&victim, &data.path().join("categories.lock"))
        .expect("this test must be able to create a symbolic link");
    let store = CategoryStore::new(data.path());
    assert!(matches!(
        store.update(|file| file.add("x", "X")),
        Err(CategoryError::NotRegularFile(_))
    ));
    assert_eq!(fs::read(&victim).unwrap(), b"keep me");
}

/// M4 success criterion (engine side): a `.zip` goes to the archives
/// folder. The name comes from `Content-Disposition`, not from the URL,
/// which is why the folder is chosen once the server has answered.
#[tokio::test]
async fn a_zip_named_by_the_server_goes_to_the_archives_folder() {
    let server = TestServer::http();
    let data = random_bytes(300_000, 4);
    server.put(
        "/download",
        FileSpec::new(data.clone()).content_disposition("attachment; filename=\"photos 2025.zip\""),
    );
    let home = tempfile::tempdir().unwrap();
    let archives = home.path().join("مضغوطات");
    let given = home.path().join("downloads");
    fs::create_dir_all(&given).unwrap();
    let categories = store_with_archives_in(home.path(), &archives)
        .load()
        .unwrap();
    let choose =
        |name: &str, url: &zajil_core::Url| categories.folder_for(name, url.host_str(), &given);
    let engine = Engine::new(Options::default()).unwrap();
    let finished = engine
        .download_choosing_until(&server.url("/download"), &choose, &Stop::new(), &mut |_| {})
        .await
        .unwrap();
    assert_same_file(&finished.file, &archives.join("photos 2025.zip"));
    assert_eq!(
        sha256_hex(&fs::read(&finished.file).unwrap()),
        sha256_hex(&data)
    );
    assert_eq!(fs::read_dir(&given).unwrap().count(), 0);
}

#[tokio::test]
async fn a_site_rule_wins_and_a_file_without_a_category_stays_in_the_given_folder() {
    let server = TestServer::http();
    server.put("/a.zip", FileSpec::new(random_bytes(1000, 5)));
    server.put("/notes.unknown", FileSpec::new(random_bytes(1000, 6)));
    let home = tempfile::tempdir().unwrap();
    let given = home.path().join("downloads");
    let from_site = home.path().join("from-the-test-server");
    let store = store_with_archives_in(home.path(), &home.path().join("archives"));
    store
        .update(|file| {
            file.add("local", "Local")?;
            let c = file.get_mut("local")?;
            c.sites.push("127.0.0.1".into());
            c.folder = Some(from_site.clone());
            Ok(())
        })
        .unwrap();
    let mut categories: CategoriesFile = store.load().unwrap();
    let choose =
        |name: &str, url: &zajil_core::Url| categories.folder_for(name, url.host_str(), &given);
    let engine = Engine::new(Options::default()).unwrap();
    let zip = engine
        .download_choosing_until(&server.url("/a.zip"), &choose, &Stop::new(), &mut |_| {})
        .await
        .unwrap();
    assert_same_file(&zip.file, &from_site.join("a.zip"));

    // Without the site rule, a file of no known type stays where it was
    // given (the folder is created).
    categories.remove("local").unwrap();
    let choose =
        |name: &str, url: &zajil_core::Url| categories.folder_for(name, url.host_str(), &given);
    let other = engine
        .download_choosing_until(
            &server.url("/notes.unknown"),
            &choose,
            &Stop::new(),
            &mut |_| {},
        )
        .await
        .unwrap();
    assert_same_file(&other.file, &given.join("notes.unknown"));
}
