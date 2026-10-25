//! The most connections at once to a site, shared by all its downloads
//! (owner's decision, 2026-10-26). Against the local test server only.

use std::time::Duration;

use zajil_core::settings::{SettingsFile, SettingsStore, SiteConnections};
use zajil_core::sites::SiteLimits;
use zajil_core::{Engine, Error, Options, Stop};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

fn engine(limit: Option<u32>) -> Engine {
    let engine = Engine::new(Options {
        connections: 4,
        min_segment_size: 64 * 1024,
        ..Options::default()
    })
    .unwrap();
    match limit {
        Some(n) => engine.with_site_limits(SiteLimits::new([("127.0.0.1", n)])),
        None => engine,
    }
}

/// Two split downloads of the same file at once: `most_concurrent` counts
/// the bodies the server sends at the same moment, both downloads together.
async fn two_at_once(limit: Option<u32>) -> usize {
    let server = TestServer::http();
    let data = random_bytes(2_000_000, 41);
    server.put("/f.bin", FileSpec::new(data.clone()).throttle(1_000_000));
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let engine = engine(limit);
    let url = server.url("/f.bin");
    let second = engine.clone();
    let (mut on_a, mut on_b) = (|_| {}, |_| {});
    let (one, two) = tokio::join!(
        engine.download(&url, a.path(), &mut on_a),
        second.download(&url, b.path(), &mut on_b),
    );
    for done in [one.unwrap(), two.unwrap()] {
        assert_eq!(
            sha256_hex(&std::fs::read(done.file).unwrap()),
            sha256_hex(&data)
        );
    }
    server.most_concurrent("/f.bin")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_limited_site_never_gets_more_connections_than_its_limit() {
    let most = two_at_once(Some(2)).await;
    assert!((1..=2).contains(&most), "{most} connections at once");
}

/// The control: without a limit the same two downloads open more, so the
/// counter is what held them above.
#[tokio::test(flavor = "multi_thread")]
async fn without_a_limit_the_same_downloads_open_more() {
    let most = two_at_once(None).await;
    assert!(most > 2, "{most} connections at once");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_limit_of_one_still_finishes_a_split_download() {
    let server = TestServer::http();
    let data = random_bytes(1_000_000, 42);
    server.put("/f.bin", FileSpec::new(data.clone()).throttle(2_000_000));
    let dir = tempfile::tempdir().unwrap();
    let done = engine(Some(1))
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(
        sha256_hex(&std::fs::read(done.file).unwrap()),
        sha256_hex(&data)
    );
    assert_eq!(server.most_concurrent("/f.bin"), 1);
}

/// A download waiting for its site's slot stops at once when asked, and
/// leaves nothing; a resume waiting stops resumable.
#[tokio::test(flavor = "multi_thread")]
async fn a_download_waiting_for_its_site_stops_when_asked() {
    let server = TestServer::http();
    server.put(
        "/slow.bin",
        FileSpec::new(random_bytes(3_000_000, 43)).throttle(300_000),
    );
    server.put("/other.bin", FileSpec::new(random_bytes(100_000, 44)));
    let engine = engine(Some(1));
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let holder = Stop::new();
    let slow = {
        let (engine, url, dir, stop) = (
            engine.clone(),
            server.url("/slow.bin"),
            a.path().to_owned(),
            holder.clone(),
        );
        tokio::spawn(async move { engine.download_until(&url, &dir, &stop, &mut |_| {}).await })
    };
    // Until the slow one has its slot and is sending.
    while server.gets("/slow.bin").is_empty() {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let stop = Stop::new();
    let later = stop.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        later.request();
    });
    let waited = engine
        .download_until(&server.url("/other.bin"), b.path(), &stop, &mut |_| {})
        .await
        .unwrap_err();
    assert!(
        matches!(waited, Error::Stopped { state_file: None }),
        "{waited:?}"
    );
    assert!(
        server.gets("/other.bin").is_empty(),
        "it asked the site without a slot"
    );
    assert_eq!(std::fs::read_dir(b.path()).unwrap().count(), 0);

    // The slow one, stopped, resumes later: while another holds the slot,
    // its resume waits, and stops resumable.
    holder.request();
    let Err(Error::Stopped {
        state_file: Some(state_file),
    }) = slow.await.unwrap()
    else {
        panic!("the slow download did not stop with a state file");
    };
    let busy = {
        let (engine, url, dir) = (
            engine.clone(),
            server.url("/slow.bin"),
            tempfile::tempdir().unwrap(),
        );
        let keep = Stop::new();
        let k = keep.clone();
        let task = tokio::spawn(async move {
            let r = engine
                .download_until(&url, dir.path(), &k, &mut |_| {})
                .await;
            drop(dir);
            r
        });
        (task, keep)
    };
    while server.gets("/slow.bin").len() < 2 {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let stop = Stop::new();
    stop.request();
    let resumed = engine
        .resume_until(&state_file, &stop, &mut |_| {})
        .await
        .unwrap_err();
    assert!(
        matches!(&resumed, Error::Stopped { state_file: Some(f) } if *f == state_file),
        "{resumed:?}"
    );
    busy.1.request();
    let _ = busy.0.await;
}

/// An engine limited to `n` on the test server's site, and that site's
/// counter, to see every slot come back.
fn limited(n: u32) -> (Engine, std::sync::Arc<tokio::sync::Semaphore>) {
    let limits = SiteLimits::new([("127.0.0.1", n)]);
    let counter = limits
        .counter_for(&zajil_core::Url::parse("http://127.0.0.1/").unwrap())
        .unwrap();
    (engine(None).with_site_limits(limits), counter)
}

/// One download with four connections on a site limited to three opens
/// exactly three: its own slot and two more. And every slot comes back.
#[tokio::test(flavor = "multi_thread")]
async fn one_split_download_opens_as_many_connections_as_the_limit() {
    let server = TestServer::http();
    let data = random_bytes(3_000_000, 45);
    server.put("/f.bin", FileSpec::new(data.clone()).throttle(500_000));
    let dir = tempfile::tempdir().unwrap();
    let (engine, counter) = limited(3);
    let done = engine
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(
        sha256_hex(&std::fs::read(done.file).unwrap()),
        sha256_hex(&data)
    );
    assert_eq!(server.most_concurrent("/f.bin"), 3);
    assert_eq!(counter.available_permits(), 3);
}

/// Review of #166: a connection that found no slot used to end for good.
/// Now it looks again: once the other download is over, this one grows to
/// two connections.
#[tokio::test(flavor = "multi_thread")]
async fn a_connection_without_a_slot_opens_when_one_comes_back() {
    let server = TestServer::http();
    let short = random_bytes(500_000, 46);
    let long = random_bytes(4_000_000, 47);
    server.put("/short.bin", FileSpec::new(short).throttle(1_000_000));
    server.put("/long.bin", FileSpec::new(long.clone()).throttle(500_000));
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (engine, counter) = limited(2);
    let one = engine.with_connections(1);
    let two = engine.with_connections(2);
    let first = {
        let (url, dir) = (server.url("/short.bin"), a.path().to_owned());
        tokio::spawn(async move { one.download(&url, &dir, &mut |_| {}).await })
    };
    while server.gets("/short.bin").is_empty() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // The long one starts on the last slot; its second connection waits.
    let done = two
        .download(&server.url("/long.bin"), b.path(), &mut |_| {})
        .await
        .unwrap();
    first.await.unwrap().unwrap();
    assert_eq!(
        sha256_hex(&std::fs::read(done.file).unwrap()),
        sha256_hex(&long)
    );
    assert_eq!(server.most_concurrent("/long.bin"), 2);
    assert_eq!(counter.available_permits(), 2);
}

/// Review of #166: a resume that waited for its slot reads its state again,
/// so one finished meanwhile by another resume is not downloaded again.
#[tokio::test(flavor = "multi_thread")]
async fn a_resume_that_waited_for_its_slot_does_not_download_a_finished_file_again() {
    let server = TestServer::http();
    let data = random_bytes(2_000_000, 48);
    server.put("/f.bin", FileSpec::new(data.clone()).throttle(1_000_000));
    let dir = tempfile::tempdir().unwrap();
    let (engine, counter) = limited(1);
    // A download stopped early, with its state file.
    let stop = Stop::new();
    let early = stop.clone();
    let err = engine
        .download_until(&server.url("/f.bin"), dir.path(), &stop, &mut |e| {
            if matches!(e, zajil_core::Event::Progress { done, .. } if done > 200_000) {
                early.request();
            }
        })
        .await
        .unwrap_err();
    let Error::Stopped {
        state_file: Some(state_file),
    } = err
    else {
        panic!("{err:?}");
    };
    // Two resumes of it at once: one waits for the other's slot.
    let (mut on_a, mut on_b) = (|_| {}, |_| {});
    let (one, two) = tokio::join!(
        engine.resume(&state_file, &mut on_a),
        engine.resume(&state_file, &mut on_b),
    );
    let finished = one.as_ref().or(two.as_ref()).expect("one resume finishes");
    assert!(
        one.is_err() || two.is_err(),
        "both finished: downloaded twice"
    );
    assert_eq!(
        sha256_hex(&std::fs::read(&finished.file).unwrap()),
        sha256_hex(&data)
    );
    // One file, never `f (1).bin` from a second whole download.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert_eq!(counter.available_permits(), 1);
}

#[test]
fn the_settings_list_is_empty_by_default_and_checked() {
    let data = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(data.path());
    assert!(store.load().unwrap().site_connections.is_empty());
    // Not written while empty: a file without limits stays as it was.
    store.update(|_| Ok(())).unwrap();
    let text = std::fs::read_to_string(store.path()).unwrap();
    assert!(!text.contains("site_connections"), "{text}");

    let ok = |site: &str, connections| SiteConnections {
        site: site.into(),
        connections,
    };
    store
        .update(|file| {
            file.site_connections = vec![ok("example.com", 2), ok("files.example.org", 1)];
            Ok(())
        })
        .unwrap();
    let file = store.load().unwrap();
    assert_eq!(file.site_connections.len(), 2);
    let limits = SiteLimits::from_settings(&file);
    let url = zajil_core::Url::parse("https://dl.example.com/a").unwrap();
    assert_eq!(limits.counter_for(&url).unwrap().available_permits(), 2);

    for (bad, why) in [
        (vec![ok("Example.com", 2)], "small letters"),
        (vec![ok("https://example.com", 2)], "not a site name"),
        (vec![ok("example.com/x", 2)], "not a site name"),
        (vec![ok("example.com:8080", 2)], "not a site name"),
        (vec![ok("*.example.com", 2)], "not a site name"),
        (vec![ok("example.com.", 2)], "not a site name"),
        (vec![ok("bücher.de", 2)], "xn--"),
        (vec![ok("example.com", 0)], "from 1 to"),
        (vec![ok("example.com", 33)], "from 1 to"),
        (vec![ok("example.com", 2), ok("example.com", 3)], "twice"),
    ] {
        let file = SettingsFile {
            site_connections: bad,
            ..SettingsFile::default()
        };
        let err = file.check().unwrap_err();
        assert!(err.contains(why), "{err}");
    }
    // Read strictly, like the rest of the file.
    assert!(
        serde_json::from_str::<SiteConnections>(
            r#"{"site": "example.com", "connections": 2, "speed": 1}"#
        )
        .is_err()
    );
    let many = SettingsFile {
        site_connections: (0..101).map(|i| ok(&format!("s{i}.test"), 1)).collect(),
        ..SettingsFile::default()
    };
    assert!(many.check().unwrap_err().contains("at most 100"));
}
