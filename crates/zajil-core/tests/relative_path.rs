//! `resume` with a relative state-file path. This changes the current
//! folder, so it is the only test in this file (one test binary = one
//! process).

use std::fs;

use zajil_core::{Engine, Error, Options};
use zajil_testserver::{FileSpec, TestServer, random_bytes};

#[tokio::test]
async fn resumes_from_a_relative_state_file_path() {
    // Review of #17: `cd <folder> && zajil resume name.zajil-state` finished
    // the download, then reported failure (`sync_dir("")`).
    let server = TestServer::http();
    let data = random_bytes(1_000_000, 50);
    server.put("/f.bin", FileSpec::new(data.clone()).cut_after(300_000, 1));
    let dir = tempfile::tempdir().unwrap();
    // One try, so the cut stops the download.
    let engine = Engine::new(Options {
        max_failures: 1,
        ..Options::default()
    })
    .unwrap();
    match engine
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
    {
        Err(Error::Interrupted { .. }) => {}
        other => panic!("expected an interruption, got {other:?}"),
    }

    std::env::set_current_dir(dir.path()).unwrap();
    let done = engine
        .resume("f.bin.zajil-state".as_ref(), &mut |_| {})
        .await
        .unwrap();

    assert!(done.file.is_absolute(), "{}", done.file.display());
    assert_eq!(fs::read(&done.file).unwrap(), data);
    assert!(!dir.path().join("f.bin.zajil-state").exists());
}
