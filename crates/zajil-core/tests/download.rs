//! Single-connection downloads against the local test server.

use std::fs;
use std::net::{Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use zajil_core::state::DownloadState;
use zajil_core::{Engine, Error, Event, Options};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

fn engine() -> Engine {
    Engine::new(Options::default()).unwrap()
}

fn quick_engine() -> Engine {
    Engine::new(Options {
        idle_timeout: Duration::from_millis(500),
        // These tests stop at the first failure, to check what is left on
        // disk and how it resumes. Retrying has its own tests (retry.rs).
        max_failures: 1,
        ..Options::default()
    })
    .unwrap()
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn downloads_50_mb_and_the_sha256_matches() {
    let server = TestServer::http();
    let data = random_bytes(50 * 1024 * 1024, 50);
    let expected = sha256_hex(&data);
    server.put("/big.bin", FileSpec::new(data));
    let dir = tempfile::tempdir().unwrap();

    let done = engine()
        .download(&server.url("/big.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap();

    assert_eq!(done.bytes, 50 * 1024 * 1024);
    assert_eq!(done.file.file_name().unwrap(), "big.bin");
    assert_eq!(sha256_hex(&fs::read(&done.file).unwrap()), expected);
    // Only the final file is left: no part or state file.
    assert_eq!(names_in(dir.path()), vec!["big.bin"]);
}

#[tokio::test]
async fn reports_events_in_order() {
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(random_bytes(300_000, 1)));
    let dir = tempfile::tempdir().unwrap();
    let mut events = Vec::new();
    engine()
        .download(&server.url("/f.bin"), dir.path(), &mut |e| events.push(e))
        .await
        .unwrap();

    assert!(matches!(
        events.first(),
        Some(Event::Started {
            total: Some(300_000),
            from: 0,
            ..
        })
    ));
    assert!(matches!(
        events.last(),
        Some(Event::Finished { bytes: 300_000, .. })
    ));
    let progress: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            Event::Progress { done, .. } => Some(*done),
            _ => None,
        })
        .collect();
    assert!(progress.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(progress.last(), Some(&300_000));
}

#[tokio::test]
async fn follows_redirects_and_names_the_file_after_the_final_url() {
    let server = TestServer::http();
    server.redirect("/start", "/middle", 301);
    server.redirect("/middle", "/files/real.iso", 302);
    server.put("/files/real.iso", FileSpec::new(vec![9u8; 1000]));
    let dir = tempfile::tempdir().unwrap();
    let done = engine()
        .download(&server.url("/start"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(done.file.file_name().unwrap(), "real.iso");
}

#[tokio::test]
async fn stops_after_too_many_redirects() {
    let server = TestServer::http();
    for i in 0..20 {
        server.redirect(&format!("/r{i}"), &format!("/r{}", i + 1), 302);
    }
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(Options {
        max_redirects: 5,
        ..Options::default()
    })
    .unwrap();
    let err = engine
        .download(&server.url("/r0"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();
    assert!(err.to_string().contains("redirects"), "{err}");
    assert_eq!(server.requests().len(), 6);
    assert!(names_in(dir.path()).is_empty());
}

#[tokio::test]
async fn dangerous_names_from_the_server_stay_in_the_folder() {
    let server = TestServer::http();
    let absolute = std::env::temp_dir().join("zajil-escape.txt");
    let long = "L".repeat(5000);
    let cases: Vec<(String, &str)> = vec![
        ("attachment; filename=\"../../x\"".into(), "x"),
        ("attachment; filename=\"..\\\\..\\\\y\"".into(), "y"),
        (
            format!("attachment; filename=\"{}\"", absolute.display()),
            "zajil-escape.txt",
        ),
        ("attachment; filename=\"CON\"".into(), "_CON"),
        ("attachment; filename=\"NUL.txt\"".into(), "_NUL.txt"),
        (
            "attachment; filename*=UTF-8''a%E2%80%AEtxt.exe".into(),
            "atxt.exe",
        ),
        (format!("attachment; filename=\"{long}\""), ""),
    ];

    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("downloads");
    fs::create_dir(&dir).unwrap();
    for (i, (header, expected)) in cases.iter().enumerate() {
        let path = format!("/f{i}");
        server.put(
            &path,
            FileSpec::new(vec![1u8; 10]).content_disposition(header),
        );
        let done = engine()
            .download(&server.url(&path), &dir, &mut |_| {})
            .await
            .unwrap_or_else(|e| panic!("{header}: {e}"));
        assert_eq!(
            done.file.parent().unwrap().canonicalize().unwrap(),
            dir.canonicalize().unwrap()
        );
        if !expected.is_empty() {
            assert_eq!(done.file.file_name().unwrap(), *expected);
        }
    }
    assert_eq!(names_in(root.path()), vec!["downloads"]);
    assert!(!absolute.exists());
}

#[tokio::test]
async fn never_overwrites_an_existing_file() {
    let server = TestServer::http();
    server.put("/a.txt", FileSpec::new(b"new".to_vec()));
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), b"old").unwrap();
    let done = engine()
        .download(&server.url("/a.txt"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(done.file.file_name().unwrap(), "a (1).txt");
    assert_eq!(fs::read(dir.path().join("a.txt")).unwrap(), b"old");
}

#[tokio::test]
async fn downloads_a_file_of_unknown_size() {
    let server = TestServer::http();
    let data = random_bytes(500_000, 3);
    server.put("/u.bin", FileSpec::new(data.clone()).without_length());
    let dir = tempfile::tempdir().unwrap();
    let done = engine()
        .download(&server.url("/u.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(fs::read(done.file).unwrap(), data);
}

#[tokio::test]
async fn works_over_ipv6() {
    let server = TestServer::http_on(SocketAddr::from((Ipv6Addr::LOCALHOST, 0)))
        .expect("IPv6 loopback is available on CI runners");
    assert!(server.url("/").starts_with("http://[::1]:"));
    let data = random_bytes(100_000, 6);
    server.put("/v6.bin", FileSpec::new(data.clone()));
    let dir = tempfile::tempdir().unwrap();
    let done = engine()
        .download(&server.url("/v6.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(fs::read(done.file).unwrap(), data);
}

/// Returns the state file named in an `Interrupted` error.
fn interrupted(err: Error) -> PathBuf {
    match err {
        Error::Interrupted { state_file, .. } => state_file,
        other => panic!("expected an interruption, got {other}"),
    }
}

#[tokio::test]
async fn a_cut_connection_leaves_a_resumable_state() {
    let server = TestServer::http();
    let data = random_bytes(1_000_000, 7);
    server.put(
        "/cut.bin",
        FileSpec::new(data.clone()).cut_after(300_000, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let err = quick_engine()
        .download(&server.url("/cut.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();
    let state_file = interrupted(err);

    let state = DownloadState::load(&state_file).unwrap();
    let done = state.bytes_done();
    assert!(done > 0 && done <= 300_000, "done = {done}");
    assert_eq!(state.content_length, Some(1_000_000));
    assert!(state.accept_ranges);
    assert!(state.validator.etag.is_some());

    // The bytes recorded as done are exactly the start of the file.
    let part = fs::read(dir.path().join("cut.bin.zajil-part")).unwrap();
    assert_eq!(&part[..done as usize], &data[..done as usize]);
    assert!(!dir.path().join("cut.bin").exists());
}

#[tokio::test]
async fn a_stalled_server_times_out_and_leaves_a_resumable_state() {
    let server = TestServer::http();
    server.put(
        "/stall.bin",
        FileSpec::new(random_bytes(200_000, 8)).stall_after(50_000),
    );
    let dir = tempfile::tempdir().unwrap();
    let err = quick_engine()
        .download(&server.url("/stall.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();
    let state = DownloadState::load(&interrupted(err)).unwrap();
    assert_eq!(state.bytes_done(), 50_000);
}

#[tokio::test]
async fn an_error_status_leaves_nothing_behind() {
    let server = TestServer::http();
    let dir = tempfile::tempdir().unwrap();
    let err = engine()
        .download(&server.url("/missing"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Status(s) if s.as_u16() == 404));
    assert!(names_in(dir.path()).is_empty());
}

#[tokio::test]
async fn rejects_unsupported_and_invalid_urls() {
    let dir = tempfile::tempdir().unwrap();
    for url in ["ftp://example.com/f", "file:///etc/passwd", "not a url"] {
        let result = engine().download(url, dir.path(), &mut |_| {}).await;
        assert!(
            matches!(
                result,
                Err(Error::UnsupportedScheme(_) | Error::InvalidUrl { .. })
            ),
            "{url}"
        );
    }
    assert!(names_in(dir.path()).is_empty());
}

#[tokio::test]
async fn credentials_in_the_url_are_never_written_to_the_state_file() {
    // Review of #15: `http://alice:s3cret@…` ended up in `.zajil-state`.
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(1_000_000, 21)).cut_after(300_000, 1),
    );
    let url = server
        .url("/f.bin")
        .replace("http://", "http://alice:s3cret@");
    let dir = tempfile::tempdir().unwrap();
    let err = quick_engine()
        .download(&url, dir.path(), &mut |_| {})
        .await
        .unwrap_err();
    let state_file = interrupted(err);

    let text = fs::read_to_string(&state_file).unwrap();
    assert!(!text.contains("s3cret"), "{text}");
    assert!(!text.contains("alice"), "{text}");
    let state = DownloadState::load(&state_file).unwrap();
    assert_eq!(state.url, server.url("/f.bin"));
    // Only that it had them (owner's decision on #28), never what they were.
    assert!(state.had_credentials);
    assert!(text.contains("\"had_credentials\": true"), "{text}");
}

#[tokio::test]
async fn a_file_that_appears_during_the_download_is_not_overwritten() {
    let server = TestServer::http();
    let data = random_bytes(500_000, 22);
    server.put("/race.bin", FileSpec::new(data.clone()));
    let dir = tempfile::tempdir().unwrap();
    let taken = dir.path().join("race.bin");

    let done = engine()
        .download(&server.url("/race.bin"), dir.path(), &mut |event| {
            // Another program takes the name while we download.
            if matches!(event, Event::Started { .. }) {
                fs::write(&taken, b"someone else's file").unwrap();
            }
        })
        .await
        .unwrap();

    assert_eq!(done.file.file_name().unwrap(), "race (1).bin");
    assert_eq!(fs::read(&done.file).unwrap(), data);
    assert_eq!(fs::read(&taken).unwrap(), b"someone else's file");
    assert_eq!(names_in(dir.path()), vec!["race (1).bin", "race.bin"]);
}

#[tokio::test]
async fn a_download_that_fails_before_any_byte_leaves_no_orphan_files() {
    // Review of #15: an empty `.zajil-part` was left behind and took the
    // name, so the next download became `f (1).bin`.
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(100_000, 30)).stall_after(0),
    );
    let dir = tempfile::tempdir().unwrap();

    let err = quick_engine()
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NoData(_)), "{err}");
    assert!(
        names_in(dir.path()).is_empty(),
        "{:?}",
        names_in(dir.path())
    );

    // The name is free again.
    server.put("/f.bin", FileSpec::new(random_bytes(100_000, 30)));
    let done = engine()
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(done.file.file_name().unwrap(), "f.bin");
}

#[tokio::test]
async fn a_download_cancelled_before_any_byte_leaves_no_orphan_files() {
    // Dropping the future is what Ctrl+C does in `zajil`.
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(100_000, 31)).stall_after(0),
    );
    let dir = tempfile::tempdir().unwrap();
    let engine = engine();
    let url = server.url("/f.bin");
    let mut started = false;
    let mut on_event = |event: Event| started |= matches!(event, Event::Started { .. });
    let download = engine.download(&url, dir.path(), &mut on_event);

    let result = tokio::time::timeout(Duration::from_millis(500), download).await;
    assert!(result.is_err(), "the download should still be waiting");
    assert!(started, "the files existed before the cancellation");
    assert!(
        names_in(dir.path()).is_empty(),
        "{:?}",
        names_in(dir.path())
    );
}

#[tokio::test]
async fn a_download_that_fails_after_some_bytes_keeps_its_files() {
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(100_000, 32)).stall_after(1),
    );
    let dir = tempfile::tempdir().unwrap();
    let err = quick_engine()
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Interrupted { .. }), "{err}");
    assert_eq!(
        names_in(dir.path()),
        vec!["f.bin.zajil-part", "f.bin.zajil-state"]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn saving_the_state_never_blocks_the_async_runtime() {
    // Review of #13: on Windows a state save can wait up to about a second
    // for an antivirus to release the file. That wait used `thread::sleep`
    // on the async thread. Here a single-threaded runtime must keep running
    // other tasks (a ticker, and the task that releases the file) while the
    // file is held. On macOS and Linux holding the file blocks nothing, so
    // this test only has teeth on Windows.
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    let server = TestServer::http();
    let data = random_bytes(1_000_000, 70);
    server.put("/f.bin", FileSpec::new(data.clone()).throttle(1_000_000));
    let dir = tempfile::tempdir().unwrap();
    let state_file = dir.path().join("f.bin.zajil-state");
    let engine = Engine::new(Options {
        save_interval: Duration::from_millis(50),
        ..Options::default()
    })
    .unwrap();

    let held: Arc<Mutex<Option<fs::File>>> = Arc::default();
    let ticks = Arc::new(AtomicU64::new(0));
    let ticker = tokio::spawn({
        let ticks = ticks.clone();
        async move {
            loop {
                tokio::time::sleep(Duration::from_millis(10)).await;
                ticks.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
    // Lets go 400 ms after the file is grabbed (at `Started`), not after the
    // test starts: in M2 more work comes before `Started` (free space,
    // allocation), and a slow machine could otherwise grab it too late and
    // never let go (review of the M2 fixes).
    let releaser = tokio::spawn({
        let held = held.clone();
        async move {
            while held.lock().unwrap().is_none() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
            held.lock().unwrap().take();
        }
    });
    let mut on_event = {
        let held = held.clone();
        move |event: Event| {
            if matches!(event, Event::Started { .. }) {
                let file = zajil_platform::fs::open_like_a_scanner(&state_file).unwrap();
                *held.lock().unwrap() = Some(file);
            }
        }
    };

    let started = Instant::now();
    let done = engine
        .download(&server.url("/f.bin"), dir.path(), &mut on_event)
        .await
        .unwrap();
    let elapsed_ms = started.elapsed().as_millis() as u64;
    ticker.abort();
    releaser.await.unwrap();

    assert_eq!(fs::read(&done.file).unwrap(), data);
    let ticks = ticks.load(Ordering::Relaxed);
    assert!(
        ticks >= elapsed_ms / 40,
        "the runtime stalled: {ticks} ticks in {elapsed_ms} ms"
    );
}

#[test]
fn cancelling_during_the_first_state_save_leaves_no_files() {
    // Review of #15, reproduced: a blocking task cannot be cancelled, so a
    // download cancelled while its first state save waited for the blocking
    // pool removed its files, and then the save wrote `.zajil-state` anyway.
    // Here the pool has one thread, kept busy so the save is still queued
    // when the download is cancelled.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let server = TestServer::http();
        server.put(
            "/f.bin",
            FileSpec::new(random_bytes(500_000, 90)).stall_after(0),
        );
        let dir = tempfile::tempdir().unwrap();
        let engine = engine();
        let url = server.url("/f.bin");

        let busy = tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_millis(400)));
        tokio::time::sleep(Duration::from_millis(20)).await;
        let cancelled = tokio::time::timeout(
            Duration::from_millis(200),
            engine.download(&url, dir.path(), &mut |_| {}),
        )
        .await;
        assert!(cancelled.is_err(), "the download should still be waiting");

        // Let the queued save run, then check nothing came back.
        busy.await.unwrap();
        tokio::task::spawn_blocking(|| ()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            names_in(dir.path()).is_empty(),
            "{:?}",
            names_in(dir.path())
        );
    });
}

#[test]
fn a_new_download_right_after_a_cancelled_one_never_loses_its_data() {
    // Review of #15, reproduced: the guard removed `.zajil-part` at once,
    // which freed the name while the cancelled download's save was still
    // queued. A second download then took the same name, and when the old
    // save finished it removed the *second* download's files.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let server = TestServer::http();
        server.put(
            "/a/f.bin",
            FileSpec::new(random_bytes(500_000, 92)).stall_after(0),
        );
        let data = random_bytes(2_000_000, 93);
        server.put("/b/f.bin", FileSpec::new(data.clone()));
        let dir = tempfile::tempdir().unwrap();
        let engine = engine();

        // A: its first save waits behind a busy blocking thread, and A is
        // cancelled meanwhile.
        let busy = tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_millis(400)));
        tokio::time::sleep(Duration::from_millis(20)).await;
        let cancelled = tokio::time::timeout(
            Duration::from_millis(150),
            engine.download(&server.url("/a/f.bin"), dir.path(), &mut |_| {}),
        )
        .await;
        assert!(cancelled.is_err());
        // A's name is still reserved by its part file.
        assert!(dir.path().join("f.bin.zajil-part").exists());

        // B starts at once, with the same name.
        let done = engine
            .download(&server.url("/b/f.bin"), dir.path(), &mut |_| {})
            .await
            .expect("the second download completes");
        assert_eq!(done.file.file_name().unwrap(), "f (1).bin");
        assert_eq!(
            sha256_hex(&fs::read(&done.file).unwrap()),
            sha256_hex(&data)
        );

        // Once every blocking task is done, nothing of A is left.
        busy.await.unwrap();
        tokio::task::spawn_blocking(|| ()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(names_in(dir.path()), vec!["f (1).bin"]);
        assert_eq!(fs::read(&done.file).unwrap(), data);
    });
}

/// Owner's decision on #28: a token in the URL (`?sig=…`) stays in the
/// state file, because resuming needs it, but that file is for its owner
/// only, and no error message shows the token.
#[tokio::test]
async fn a_token_in_the_url_is_kept_privately_and_never_shown() {
    const TOKEN: &str = "SECRET-TOKEN";
    let server = TestServer::http();
    server.put(
        "/cut.bin",
        FileSpec::new(random_bytes(1_000_000, 33)).cut_after(300_000, 1),
    );
    server.put(
        "/stall.bin",
        FileSpec::new(random_bytes(1_000_000, 34)).stall_after(1_000),
    );
    let dir = tempfile::tempdir().unwrap();

    // Cut, and stalled (a read timeout from the HTTP library).
    for path in ["/cut.bin", "/stall.bin"] {
        let url = format!("{}?sig={TOKEN}#{TOKEN}", server.url(path));
        let err = quick_engine()
            .download(&url, dir.path(), &mut |_| {})
            .await
            .unwrap_err();
        assert!(!format!("{err} {err:?}").contains(TOKEN), "{err:?}");
        let state_file = interrupted(err);
        let text = fs::read_to_string(&state_file).unwrap();
        assert!(text.contains(TOKEN), "resuming needs it: {text}");
        assert!(zajil_platform::fs::only_owner_can_access(&state_file).unwrap());
    }

    // No server there, and a URL that does not parse.
    for url in [
        format!("http://127.0.0.1:1/f.bin?sig={TOKEN}"),
        format!("http://u:p@127.0.0.1:99999/f.bin?sig={TOKEN}"),
        format!("http://127.0.0.1:99999/f.bin#{TOKEN}"),
    ] {
        let err = quick_engine()
            .download(&url, dir.path(), &mut |_| {})
            .await
            .unwrap_err();
        let shown = err.to_string();
        assert!(!shown.contains(TOKEN), "{shown}");
        assert!(shown.contains("127.0.0.1"), "{shown}");
    }
}
