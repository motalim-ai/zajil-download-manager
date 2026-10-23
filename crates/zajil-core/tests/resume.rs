//! Resuming from the state file against the local test server.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use zajil_core::state::{DownloadState, StateError};
use zajil_core::{Engine, Error, Event, Options, RestartReason};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

const SIZE: usize = 10 * 1024 * 1024;
const THIRTY_PERCENT: u64 = (SIZE as u64) * 3 / 10;

/// These tests are about resuming on one connection (M1); resuming a
/// download of several segments is tested in `parallel.rs`.
fn engine() -> Engine {
    Engine::new(Options {
        connections: 1,
        // These tests stop at the first failure, to check what is left on
        // disk and how it resumes. Retrying has its own tests (retry.rs).
        max_failures: 1,
        ..Options::default()
    })
    .unwrap()
}

/// Starts a download that the server cuts, and returns its state file.
async fn interrupted_download(server: &TestServer, path: &str, dir: &Path) -> PathBuf {
    match engine().download(&server.url(path), dir, &mut |_| {}).await {
        Err(Error::Interrupted { state_file, .. }) => state_file,
        other => panic!("expected an interruption, got {other:?}"),
    }
}

async fn resume(state_file: &Path) -> (Result<zajil_core::Finished, Error>, Vec<Event>) {
    let mut events = Vec::new();
    let result = engine().resume(state_file, &mut |e| events.push(e)).await;
    (result, events)
}

fn restarts(events: &[Event]) -> Vec<RestartReason> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Restarted { reason } => Some(*reason),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn resumes_after_a_cut_at_30_percent_without_asking_twice() {
    let server = TestServer::http();
    let data = random_bytes(SIZE, 30);
    let spec = FileSpec::new(data.clone()).cut_after(THIRTY_PERCENT, 1);
    let etag = spec.current_etag().unwrap().to_owned();
    server.put("/f.bin", spec);
    let dir = tempfile::tempdir().unwrap();

    let state_file = interrupted_download(&server, "/f.bin", dir.path()).await;
    let on_disk = DownloadState::load(&state_file).unwrap().bytes_done();
    assert!(on_disk > 0 && on_disk <= THIRTY_PERCENT);

    let (result, events) = resume(&state_file).await;
    let done = result.unwrap();
    assert_eq!(
        sha256_hex(&fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );
    assert!(restarts(&events).is_empty());
    assert!(matches!(events[0], Event::Started { from, .. } if from == on_disk));

    // The second request asked only for the bytes we did not have.
    let gets = server.gets("/f.bin");
    assert_eq!(gets.len(), 2);
    assert_eq!(gets[0].range, None);
    assert_eq!(gets[1].range, Some(format!("bytes={on_disk}-")));
    assert_eq!(gets[1].if_range.as_deref(), Some(etag.as_str()));
    assert_eq!(gets[1].status, 206);
    assert_eq!(gets[1].bytes_sent, SIZE as u64 - on_disk);
    assert!(!state_file.exists());
}

#[tokio::test]
async fn starts_over_when_the_file_changed_on_the_server() {
    let server = TestServer::http();
    let old = random_bytes(SIZE, 1);
    server.put("/f.bin", FileSpec::new(old).cut_after(THIRTY_PERCENT, 1));
    let dir = tempfile::tempdir().unwrap();
    let state_file = interrupted_download(&server, "/f.bin", dir.path()).await;

    // New version, new ETag, different size.
    let new = random_bytes(SIZE + 12345, 2);
    server.put("/f.bin", FileSpec::new(new.clone()));
    let (result, events) = resume(&state_file).await;
    let done = result.unwrap();

    assert_eq!(restarts(&events), vec![RestartReason::FileChanged]);
    assert_eq!(fs::read(&done.file).unwrap(), new);
    // One more request: the If-Range failed, so the server sent the new file
    // whole.
    let gets = server.gets("/f.bin");
    assert_eq!(gets.len(), 2);
    assert_eq!(gets[1].status, 200);
}

#[tokio::test]
async fn starts_over_when_the_file_changed_with_the_same_size() {
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(SIZE, 3)).cut_after(THIRTY_PERCENT, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = interrupted_download(&server, "/f.bin", dir.path()).await;

    let new = random_bytes(SIZE, 4);
    server.put("/f.bin", FileSpec::new(new.clone()));
    let (result, events) = resume(&state_file).await;
    assert_eq!(restarts(&events), vec![RestartReason::FileChanged]);
    assert_eq!(fs::read(result.unwrap().file).unwrap(), new);
}

#[tokio::test]
async fn uses_a_strong_last_modified_when_there_is_no_etag() {
    let server = TestServer::http();
    let data = random_bytes(SIZE, 5);
    // The response is dated an hour after the last change: strong.
    server.put(
        "/f.bin",
        FileSpec::new(data.clone())
            .etag(None)
            .last_modified(Some(zajil_testserver::DEFAULT_LAST_MODIFIED))
            .date("Wed, 21 Oct 2026 08:28:00 GMT")
            .cut_after(THIRTY_PERCENT, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = interrupted_download(&server, "/f.bin", dir.path()).await;
    let (result, events) = resume(&state_file).await;
    assert_eq!(fs::read(result.unwrap().file).unwrap(), data);
    assert!(restarts(&events).is_empty());
    let gets = server.gets("/f.bin");
    assert_eq!(
        gets[1].if_range.as_deref(),
        Some(zajil_testserver::DEFAULT_LAST_MODIFIED)
    );
    assert_eq!(gets[1].status, 206);
}

#[tokio::test]
async fn a_server_that_answers_200_to_range_starts_over_without_mixing() {
    let server = TestServer::http();
    let data = random_bytes(SIZE, 6);
    server.put(
        "/f.bin",
        FileSpec::new(data.clone())
            .without_ranges()
            .cut_after(THIRTY_PERCENT, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = interrupted_download(&server, "/f.bin", dir.path()).await;
    assert!(!DownloadState::load(&state_file).unwrap().accept_ranges);

    let (result, events) = resume(&state_file).await;
    assert_eq!(restarts(&events), vec![RestartReason::RangesIgnored]);
    let file = result.unwrap().file;
    assert_eq!(sha256_hex(&fs::read(file).unwrap()), sha256_hex(&data));
    let gets = server.gets("/f.bin");
    assert!(gets[1].range.is_some());
    assert_eq!(gets[1].status, 200);
}

#[tokio::test]
async fn a_damaged_state_file_is_refused_and_nothing_is_touched() {
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(SIZE, 7)).cut_after(THIRTY_PERCENT, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = interrupted_download(&server, "/f.bin", dir.path()).await;
    let part = dir.path().join("f.bin.zajil-part");
    let part_before = fs::read(&part).unwrap();

    let text = fs::read_to_string(&state_file).unwrap();
    let damaged = text.replacen("\"done\": ", "\"done\": 1", 1);
    assert_ne!(text, damaged);
    fs::write(&state_file, damaged).unwrap();

    let requests_before = server.requests().len();
    let (result, _) = resume(&state_file).await;
    let err = result.unwrap_err();
    assert!(
        matches!(err, Error::State(StateError::ChecksumMismatch)),
        "{err}"
    );
    assert!(err.to_string().contains("damaged"));
    assert_eq!(server.requests().len(), requests_before);
    assert_eq!(fs::read(&part).unwrap(), part_before);
}

#[tokio::test]
async fn refuses_a_download_that_another_process_holds() {
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(SIZE, 8)).cut_after(THIRTY_PERCENT, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = interrupted_download(&server, "/f.bin", dir.path()).await;

    let holder = OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.path().join("f.bin.zajil-part"))
        .unwrap();
    holder.try_lock().unwrap();
    let (result, _) = resume(&state_file).await;
    assert!(matches!(result, Err(Error::Locked(_))));
    drop(holder);

    let (result, _) = resume(&state_file).await;
    result.unwrap();
}

#[tokio::test]
async fn starts_over_when_the_part_file_is_missing() {
    let server = TestServer::http();
    let data = random_bytes(SIZE, 9);
    server.put(
        "/f.bin",
        FileSpec::new(data.clone()).cut_after(THIRTY_PERCENT, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = interrupted_download(&server, "/f.bin", dir.path()).await;
    fs::remove_file(dir.path().join("f.bin.zajil-part")).unwrap();

    let (result, events) = resume(&state_file).await;
    assert_eq!(restarts(&events), vec![RestartReason::PartFileMissing]);
    assert_eq!(fs::read(result.unwrap().file).unwrap(), data);
}

#[tokio::test]
async fn can_be_interrupted_and_resumed_several_times() {
    let server = TestServer::http();
    let data = random_bytes(SIZE, 10);
    server.put(
        "/f.bin",
        FileSpec::new(data.clone()).cut_after(1_000_000, 3),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = interrupted_download(&server, "/f.bin", dir.path()).await;

    let mut last_done = DownloadState::load(&state_file).unwrap().bytes_done();
    for _ in 0..2 {
        let (result, _) = resume(&state_file).await;
        assert!(matches!(result, Err(Error::Interrupted { .. })));
        let done = DownloadState::load(&state_file).unwrap().bytes_done();
        assert!(done > last_done, "no progress: {done} <= {last_done}");
        last_done = done;
    }
    let (result, _) = resume(&state_file).await;
    assert_eq!(fs::read(result.unwrap().file).unwrap(), data);
}

#[tokio::test]
async fn falls_back_to_the_original_url_when_the_final_one_expired() {
    let server = TestServer::http();
    let data = random_bytes(SIZE, 11);
    server.redirect("/start", "/cdn/token1/f.bin", 302);
    server.put(
        "/cdn/token1/f.bin",
        FileSpec::new(data.clone())
            .etag(Some("\"same\""))
            .cut_after(THIRTY_PERCENT, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = interrupted_download(&server, "/start", dir.path()).await;

    // The signed link expired; the original URL now leads to a new one.
    server.put(
        "/cdn/token1/f.bin",
        FileSpec::new(Vec::new()).forbid_after(0),
    );
    server.redirect("/start", "/cdn/token2/f.bin", 302);
    server.put(
        "/cdn/token2/f.bin",
        FileSpec::new(data.clone()).etag(Some("\"same\"")),
    );

    let (result, events) = resume(&state_file).await;
    assert_eq!(fs::read(result.unwrap().file).unwrap(), data);
    assert!(restarts(&events).is_empty());
    assert_eq!(server.gets("/cdn/token2/f.bin")[0].status, 206);
}

/// Creates a symbolic link, or reports that this machine cannot. On CI
/// (where `CI` is set) that is a failure, so the test cannot silently stop
/// testing anything.
fn symlinks_available(target: &Path, link: &Path) -> bool {
    match zajil_platform::fs::symlink_file(target, link) {
        Ok(()) => true,
        Err(err) if std::env::var_os("CI").is_none() => {
            eprintln!("skipping: cannot create symbolic links here: {err}");
            false
        }
        Err(err) => panic!("CI must be able to create symbolic links: {err}"),
    }
}

/// An interrupted download in `<root>/downloads`, and a file outside that
/// folder that an attacker would like overwritten.
async fn interrupted_next_to_a_victim(
    server: &TestServer,
) -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(1_000_000, 40)).cut_after(300_000, 1),
    );
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("downloads");
    fs::create_dir(&dir).unwrap();
    let state_file = interrupted_download(server, "/f.bin", &dir).await;
    let victim = root.path().join("victim.txt");
    fs::write(&victim, b"precious").unwrap();
    (root, dir, state_file, victim)
}

#[tokio::test]
async fn refuses_a_part_file_that_is_a_symbolic_link_out_of_the_folder() {
    // Review of #17, reproduced: `.zajil-part` replaced by a link to a file
    // outside the download folder. Before the fix, resume truncated and
    // overwrote `victim.txt` with 1,000,000 bytes from the server.
    let server = TestServer::http();
    let (_root, dir, state_file, victim) = interrupted_next_to_a_victim(&server).await;
    let part = dir.join("f.bin.zajil-part");
    fs::remove_file(&part).unwrap();
    if !symlinks_available(&victim, &part) {
        return;
    }
    let requests_before = server.requests().len();

    let (result, _) = resume(&state_file).await;

    assert!(
        matches!(result, Err(Error::NotRegularFile(_))),
        "{result:?}"
    );
    assert_eq!(fs::read(&victim).unwrap(), b"precious");
    assert_eq!(server.requests().len(), requests_before);
    assert!(!dir.join("f.bin").exists());
}

#[tokio::test]
async fn refuses_a_part_file_that_is_a_hard_link_to_a_file_outside() {
    let server = TestServer::http();
    let (_root, dir, state_file, victim) = interrupted_next_to_a_victim(&server).await;
    let part = dir.join("f.bin.zajil-part");
    fs::remove_file(&part).unwrap();
    fs::hard_link(&victim, &part).unwrap();

    let (result, _) = resume(&state_file).await;

    assert!(
        matches!(result, Err(Error::NotRegularFile(_))),
        "{result:?}"
    );
    assert_eq!(fs::read(&victim).unwrap(), b"precious");
}

#[tokio::test]
async fn refuses_a_state_file_that_is_a_symbolic_link() {
    let server = TestServer::http();
    let (root, dir, state_file, _victim) = interrupted_next_to_a_victim(&server).await;
    let elsewhere = root.path().join("elsewhere.zajil-state");
    fs::rename(&state_file, &elsewhere).unwrap();
    if !symlinks_available(&elsewhere, &state_file) {
        return;
    }
    let part_before = fs::read(dir.join("f.bin.zajil-part")).unwrap();

    let (result, _) = resume(&state_file).await;

    assert!(
        matches!(result, Err(Error::State(StateError::NotRegularFile(_)))),
        "{result:?}"
    );
    assert_eq!(fs::read(dir.join("f.bin.zajil-part")).unwrap(), part_before);
}

#[tokio::test]
async fn a_temporary_state_file_planted_as_a_link_is_not_followed() {
    let server = TestServer::http();
    let (_root, dir, state_file, victim) = interrupted_next_to_a_victim(&server).await;
    let tmp = dir.join("f.bin.zajil-state.tmp");
    if !symlinks_available(&victim, &tmp) {
        return;
    }

    let (result, _) = resume(&state_file).await;

    result.unwrap();
    assert_eq!(fs::read(&victim).unwrap(), b"precious");
    assert!(dir.join("f.bin").exists());
}

#[tokio::test]
async fn a_weak_etag_is_never_trusted_to_continue_a_changed_file() {
    // Review of #17: with a weak ETag the engine sent `If-Range` with the
    // date, so a server whose file changed (same weak ETag, same date)
    // answered 206 and old and new content were mixed.
    let server = TestServer::http();
    let old = random_bytes(SIZE, 60);
    server.put(
        "/f.bin",
        FileSpec::new(old)
            .etag(Some("W/\"same\""))
            .cut_after(THIRTY_PERCENT, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = interrupted_download(&server, "/f.bin", dir.path()).await;

    let new = random_bytes(SIZE, 61);
    server.put(
        "/f.bin",
        FileSpec::new(new.clone()).etag(Some("W/\"same\"")),
    );
    let (result, events) = resume(&state_file).await;

    assert_eq!(restarts(&events), vec![RestartReason::NoValidator]);
    assert_eq!(fs::read(result.unwrap().file).unwrap(), new);
    let resumed = &server.gets("/f.bin")[1];
    assert_eq!(resumed.range, None, "no partial request at all");
    assert_eq!(resumed.if_range, None);
    assert_eq!(resumed.status, 200);
}

#[tokio::test]
async fn a_last_modified_in_the_same_second_as_the_response_is_not_trusted() {
    // RFC 9110 §8.8.2.2: Last-Modified equal to Date is a weak validator. A
    // file changed again in that second keeps the same Last-Modified.
    const SECOND: &str = "Thu, 22 Oct 2026 10:00:00 GMT";
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(SIZE, 62))
            .etag(None)
            .last_modified(Some(SECOND))
            .date(SECOND)
            .cut_after(THIRTY_PERCENT, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = interrupted_download(&server, "/f.bin", dir.path()).await;
    let saved = DownloadState::load(&state_file).unwrap();
    assert_eq!(saved.validator.date.as_deref(), Some(SECOND));

    let new = random_bytes(SIZE, 63);
    server.put(
        "/f.bin",
        FileSpec::new(new.clone())
            .etag(None)
            .last_modified(Some(SECOND))
            .date(SECOND),
    );
    let (result, events) = resume(&state_file).await;

    assert_eq!(restarts(&events), vec![RestartReason::NoValidator]);
    assert_eq!(fs::read(result.unwrap().file).unwrap(), new);
    let resumed = &server.gets("/f.bin")[1];
    assert_eq!(resumed.range, None, "no partial request at all");
    assert_eq!(resumed.if_range, None);
}

#[tokio::test]
async fn a_long_name_that_needed_a_number_can_still_be_resumed() {
    // Review of #17, reproduced: a 197–200 byte name from the server, with a
    // file of that name already in the folder, became `<name> (1).zip` at
    // 204 bytes; `resume` then refused the state file Zajil had written
    // ("this is not a Zajil state file").
    for length in [197, 198, 200] {
        let name = format!("{}.zip", "a".repeat(length - 4));
        let server = TestServer::http();
        let data = random_bytes(SIZE, 80);
        server.put(
            "/f",
            FileSpec::new(data.clone())
                .content_disposition(&format!("attachment; filename=\"{name}\""))
                .cut_after(THIRTY_PERCENT, 1),
        );
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(&name), b"an older download").unwrap();

        let state_file = interrupted_download(&server, "/f", dir.path()).await;
        let state_name = state_file.file_name().unwrap().to_str().unwrap();
        assert!(
            state_name.len() <= 200 + ".zajil-state".len(),
            "{length}: {}",
            state_name.len()
        );

        let (result, _) = resume(&state_file).await;
        let done = result.unwrap_or_else(|e| panic!("{length}: {e}"));
        let final_name = done.file.file_name().unwrap().to_str().unwrap();
        assert!(final_name.len() <= 200, "{length}: {}", final_name.len());
        assert!(final_name.ends_with(" (1).zip"), "{final_name}");
        assert_eq!(fs::read(&done.file).unwrap(), data);
        assert_eq!(
            fs::read(dir.path().join(&name)).unwrap(),
            b"an older download"
        );
    }
}

#[tokio::test]
async fn an_unsafe_state_file_name_is_refused_with_the_real_reason() {
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(SIZE, 81)).cut_after(THIRTY_PERCENT, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = interrupted_download(&server, "/f.bin", dir.path()).await;
    let long = dir
        .path()
        .join(format!("{}.bin.zajil-state", "x".repeat(210)));
    fs::copy(&state_file, &long).unwrap();

    let (result, _) = resume(&long).await;
    let err = result.unwrap_err();
    assert!(matches!(err, Error::UnsafeName { .. }), "{err}");
    let message = err.to_string();
    assert!(message.contains("200-byte limit"), "{message}");
    assert!(!message.contains("not a Zajil state file"), "{message}");
}

#[tokio::test]
async fn an_absurd_date_from_the_server_is_ignored_without_panicking() {
    // Review of #17, reproduced: `Date` with year 9223372036854775807 and no
    // ETag made `http_date` overflow (a panic in debug builds) on resume.
    let server = TestServer::http();
    let data = random_bytes(SIZE, 82);
    server.put(
        "/f.bin",
        FileSpec::new(data.clone())
            .etag(None)
            .date("Sun, 06 Nov 9223372036854775807 08:49:37 GMT")
            .cut_after(THIRTY_PERCENT, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = interrupted_download(&server, "/f.bin", dir.path()).await;

    let (result, events) = resume(&state_file).await;

    // The date cannot prove anything, so the download starts over.
    assert_eq!(restarts(&events), vec![RestartReason::NoValidator]);
    assert_eq!(fs::read(result.unwrap().file).unwrap(), data);
    assert_eq!(server.gets("/f.bin")[1].range, None);
}

#[tokio::test]
async fn a_windows_reserved_state_file_name_is_refused_with_the_exact_name() {
    // Final review: the reason said "contains characters Zajil never
    // writes". The check runs before any file is touched, so these paths do
    // not need to exist (and on Windows could not be created).
    let dir = tempfile::tempdir().unwrap();
    for (file, device) in [
        ("CON.zajil-state", "CON"),
        ("nul .txt.zajil-state", "nul"),
        ("COM1.tar.gz.zajil-state", "COM1"),
        ("lpt9.zajil-state", "lpt9"),
    ] {
        let (result, _) = resume(&dir.path().join(file)).await;
        let err = result.unwrap_err();
        assert!(matches!(err, Error::UnsafeName { .. }), "{file}: {err}");
        let message = err.to_string();
        assert!(
            message.contains(&format!("`{device}` is a device name Windows reserves")),
            "{file}: {message}"
        );
    }
    // Other unclean names say what Zajil would have written instead.
    let (result, _) = resume(&dir.path().join("a\u{200B}b.zajil-state")).await;
    let message = result.unwrap_err().to_string();
    assert!(message.contains("Zajil would write \"ab\""), "{message}");
}

/// Owner decision on #28: resuming a download that needed a user name and
/// password gives a clear error instead of a bare 401 (the state file never
/// keeps them).
#[tokio::test]
async fn resuming_a_download_that_needed_credentials_says_so() {
    let server = TestServer::http();
    let data = random_bytes(1_000_000, 95);
    // "Basic " and the base64 of "u:p".
    let spec = || {
        FileSpec::new(data.clone())
            .etag(Some("\"same\""))
            .require_authorization("Basic dTpw")
    };
    server.put("/f.bin", spec().cut_after(300_000, 1));
    let dir = tempfile::tempdir().unwrap();
    let url = server.url("/f.bin").replacen("http://", "http://u:p@", 1);
    let state_file = match engine().download(&url, dir.path(), &mut |_| {}).await {
        Err(Error::Interrupted { state_file, .. }) => state_file,
        other => panic!("expected an interruption, got {other:?}"),
    };

    let (result, _) = resume(&state_file).await;
    match result.unwrap_err() {
        Error::CredentialsNeeded {
            url,
            state_file: reported,
        } => {
            assert_eq!(url, server.url("/f.bin"), "no user name or password in it");
            assert_eq!(reported, state_file);
        }
        other => panic!("expected CredentialsNeeded, got {other:?}"),
    }
    // Nothing was touched: the download can still be resumed by hand.
    assert!(state_file.exists());
    assert!(dir.path().join("f.bin.zajil-part").exists());
}

/// The same when the resume has to start over (here the `.zajil-part` is
/// gone): the restart's request is refused with 401 too (review of #29:
/// this path had no test).
#[tokio::test]
async fn a_restart_that_needs_credentials_says_so() {
    let server = TestServer::http();
    let data = random_bytes(1_000_000, 96);
    let spec = || {
        FileSpec::new(data.clone())
            .etag(Some("\"same\""))
            .require_authorization("Basic dTpw")
    };
    server.put("/f.bin", spec().cut_after(300_000, 1));
    let dir = tempfile::tempdir().unwrap();
    let url = server.url("/f.bin").replacen("http://", "http://u:p@", 1);
    let state_file = match engine().download(&url, dir.path(), &mut |_| {}).await {
        Err(Error::Interrupted { state_file, .. }) => state_file,
        other => panic!("expected an interruption, got {other:?}"),
    };
    fs::remove_file(dir.path().join("f.bin.zajil-part")).unwrap();

    let (result, events) = resume(&state_file).await;
    assert_eq!(restarts(&events), [RestartReason::PartFileMissing]);
    assert!(
        matches!(result, Err(Error::CredentialsNeeded { .. })),
        "{result:?}"
    );
}

/// Owner's decision on #28: a `401` while resuming a download that never had
/// a user name or password (an expired signed link, an expired session) is
/// a plain `401`, not a request for a password it never had.
#[tokio::test]
async fn a_401_for_a_download_without_credentials_does_not_ask_for_a_password() {
    let server = TestServer::http();
    let data = random_bytes(1_000_000, 97);
    let spec = || FileSpec::new(data.clone()).etag(Some("\"same\""));
    server.put("/f.bin", spec().cut_after(300_000, 1));
    let dir = tempfile::tempdir().unwrap();
    let state_file = match engine()
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
    {
        Err(Error::Interrupted { state_file, .. }) => state_file,
        other => panic!("expected an interruption, got {other:?}"),
    };
    assert!(!DownloadState::load(&state_file).unwrap().had_credentials);
    let text = fs::read_to_string(&state_file).unwrap();
    assert!(text.contains("\"had_credentials\": false"), "{text}");

    server.put("/f.bin", spec().require_authorization("Basic dTpw"));
    let (result, _) = resume(&state_file).await;
    let err = result.unwrap_err();
    assert!(
        matches!(err, Error::Status(status) if status.as_u16() == 401),
        "{err:?}"
    );
    assert!(!err.to_string().contains("password"), "{err}");
    assert!(state_file.exists());
}
