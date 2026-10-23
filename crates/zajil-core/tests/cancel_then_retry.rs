//! The review's experiment for #15: a new download is cancelled while its
//! first state save is slow (an antivirus scanning the file), and a second
//! download of the same name starts at once. The hook is global, so this is
//! the only test in this file (one test binary = one process).

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use zajil_core::state::test_hooks;
use zajil_core::{Engine, Options};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

#[tokio::test]
async fn cancel_then_retry_at_once_never_loses_the_second_download() {
    // Only the first save (download A's) is slow: 500 ms.
    static SAVES: AtomicUsize = AtomicUsize::new(0);
    test_hooks::set_after_temporary_written(Some(Box::new(|_| {
        if SAVES.fetch_add(1, Ordering::SeqCst) == 0 {
            std::thread::sleep(Duration::from_millis(500));
        }
    })));

    let server = TestServer::http();
    server.put(
        "/a/f.bin",
        FileSpec::new(random_bytes(500_000, 94)).stall_after(0),
    );
    let data = random_bytes(2 * 1024 * 1024, 95);
    server.put("/b/f.bin", FileSpec::new(data.clone()));
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(Options::default()).unwrap();

    // A is cancelled at 100 ms, during its slow first save.
    let cancelled = tokio::time::timeout(
        Duration::from_millis(100),
        engine.download(&server.url("/a/f.bin"), dir.path(), &mut |_| {}),
    )
    .await;
    assert!(cancelled.is_err());

    // B starts at once, with the same name, and must keep its data.
    let done = engine
        .download(&server.url("/b/f.bin"), dir.path(), &mut |_| {})
        .await
        .expect("the second download completes");
    assert_eq!(
        sha256_hex(&fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );

    // After A's save is over, only B's file is left, and it is intact.
    tokio::time::sleep(Duration::from_millis(800)).await;
    test_hooks::set_after_temporary_written(None);
    let mut left: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    let name = done.file.file_name().unwrap().to_str().unwrap().to_owned();
    assert_eq!(left, vec![name]);
    assert_eq!(
        sha256_hex(&fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );
}
