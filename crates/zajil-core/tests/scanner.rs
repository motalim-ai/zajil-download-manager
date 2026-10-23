//! A new download cancelled while an antivirus holds its first state file.
//! The hook is global, so this is the only test in this file (one test
//! binary = one process).

use std::fs;
use std::time::Duration;

use zajil_core::state::test_hooks;
use zajil_core::{Engine, Options};
use zajil_testserver::{FileSpec, TestServer, random_bytes};

#[tokio::test]
async fn cancelling_while_an_antivirus_holds_the_first_state_file_leaves_no_files() {
    // Windows Defender scans a file when it is closed. Here the "scanner"
    // opens the new `.zajil-state.tmp` without FILE_SHARE_DELETE as soon as
    // it is written, keeps it for 600 ms, and the scan itself takes 300 ms.
    // On Windows the rename then waits in its retry loop; on macOS and Linux
    // only the 300 ms scan delays it. The download is cancelled at 150 ms,
    // while that first save is still running on a blocking thread.
    test_hooks::set_after_temporary_written(Some(Box::new(|tmp| {
        if let Ok(held) = zajil_platform::fs::open_like_a_scanner(tmp) {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(600));
                drop(held);
            });
        }
        std::thread::sleep(Duration::from_millis(300));
    })));

    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(500_000, 91)).stall_after(0),
    );
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(Options::default()).unwrap();
    let url = server.url("/f.bin");

    let cancelled = tokio::time::timeout(
        Duration::from_millis(150),
        engine.download(&url, dir.path(), &mut |_| {}),
    )
    .await;
    assert!(cancelled.is_err(), "the download should still be saving");

    // Wait for the scan, the retries and the save to finish.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    test_hooks::set_after_temporary_written(None);
    let left: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "{left:?}");
}
