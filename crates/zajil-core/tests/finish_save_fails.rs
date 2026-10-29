//! Every byte is written, but the save that records it before the rename
//! fails (on a full disk: CI on Windows, where a "full" NTFS volume still
//! took the data and refused only that save). The download stops resumable
//! from the state file saved last, and resumes whole, with one connection
//! or several.
//!
//! The hook is global (`state::test_hooks`), so this test has a binary of
//! its own.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use zajil_core::state::{DownloadState, test_hooks};
use zajil_core::{Engine, Error, Event, Options};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

const SIZE: usize = 4 << 20;

#[tokio::test]
async fn a_failed_last_save_stops_the_download_which_resumes_whole() {
    let server = TestServer::http();
    let data = random_bytes(SIZE, 313);
    // Set once every byte is written: from then on, each save loses its
    // temporary file before it replaces the state file, so it fails.
    let failing = Arc::new(AtomicBool::new(false));
    let hook = failing.clone();
    test_hooks::set_after_temporary_written(Some(Box::new(move |tmp: &Path| {
        if hook.load(Ordering::SeqCst) {
            std::fs::remove_file(tmp).unwrap();
        }
    })));

    for connections in [1, 4] {
        let path = format!("/f{connections}.bin");
        server.put(&path, FileSpec::new(data.clone()));
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::new(Options {
            connections,
            ..Options::default()
        })
        .unwrap();
        failing.store(false, Ordering::SeqCst);

        let result = engine
            .download(&server.url(&path), dir.path(), &mut |event| {
                if let Event::Progress { done, .. } = event
                    && done == SIZE as u64
                {
                    failing.store(true, Ordering::SeqCst);
                }
            })
            .await;
        assert!(
            failing.load(Ordering::SeqCst),
            "{connections}: never complete"
        );
        let (state_file, reason) = match result {
            Err(Error::Interrupted { state_file, reason }) => (state_file, reason),
            other => panic!("{connections}: expected an interruption, got {other:?}"),
        };
        assert!(reason.starts_with("could not finish the file"), "{reason}");
        // The state file saved last is whole, and the part file is there.
        let state = DownloadState::load(&state_file).unwrap();
        assert!(state.bytes_done() < SIZE as u64, "{connections}");
        assert!(!dir.path().join(format!("f{connections}.bin")).exists());

        failing.store(false, Ordering::SeqCst);
        let done = engine
            .resume(&state_file, &mut |_| {})
            .await
            .unwrap_or_else(|e| panic!("{connections}: resuming: {e}"));
        assert_eq!(
            sha256_hex(&std::fs::read(&done.file).unwrap()),
            sha256_hex(&data),
            "{connections}"
        );
        assert!(!state_file.exists(), "{connections}");
    }
    test_hooks::set_after_temporary_written(None);
}
