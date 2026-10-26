//! #163: a download that starts over from byte 0 empties its part file.
//! A crash right after that must not leave the old state, which counted
//! the old bytes as done, over a file of zeros: a later resume the server
//! allows would keep them, and the file would end whole in size but wrong,
//! with nothing said. Its own test file: the crash is one switch for the
//! whole process (`engine_test_hooks`).

use std::fs;
use std::sync::atomic::Ordering;

use zajil_core::engine_test_hooks::CRASH_AFTER_ALLOCATING;
use zajil_core::{Engine, Error, Options};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

#[tokio::test]
async fn a_crash_as_a_download_starts_over_never_leaves_zeros_counted_as_done() {
    let data = random_bytes(1_000_000, 163);
    let etag = Some("\"same\"");
    // 1. Cut at 400 000 bytes: a resumable download, with a state.
    // 2. Then a server that answers the resume's range with the whole
    //    file (the same version): the download starts over from 0.
    // 3. Then a server that honours ranges again.
    let honours = FileSpec::new(data.clone()).etag(etag);
    let ignores = FileSpec::new(data.clone())
        .etag(etag)
        .ignore_ranges()
        .replaced_after(1, honours);
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(data.clone())
            .etag(etag)
            .cut_after(400_000, 1)
            .replaced_after(1, ignores),
    );
    let engine = Engine::new(Options {
        connections: 1,
        max_failures: 1,
        ..Options::default()
    })
    .unwrap();
    let dir = tempfile::tempdir().unwrap();

    let err = engine
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();
    let Error::Interrupted { state_file, .. } = err else {
        panic!("{err}");
    };

    // The resume starts over, and «crashes» once the part is emptied.
    CRASH_AFTER_ALLOCATING.store(true, Ordering::SeqCst);
    let crashed = engine.resume(&state_file, &mut |_| {}).await;
    assert!(
        matches!(&crashed, Err(Error::Io(e)) if e.to_string().contains("a crash")),
        "{crashed:?}"
    );
    assert!(
        !CRASH_AFTER_ALLOCATING.load(Ordering::SeqCst),
        "the crash was not reached"
    );

    // After the crash, a resume the server allows ends with the real file.
    let finished = engine.resume(&state_file, &mut |_| {}).await.unwrap();
    assert_eq!(
        sha256_hex(&fs::read(&finished.file).unwrap()),
        sha256_hex(&data),
        "the file is whole in size but not the server's: zeros were kept as done"
    );
}
