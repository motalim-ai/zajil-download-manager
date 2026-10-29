//! A download moved while paused («Download file info» started it in the
//! background, then the user chose another folder and name) resumes where
//! it was, from its new place.

use std::fs;

use zajil_core::state::DownloadState;
use zajil_core::{Engine, Error, Options, relocate};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

const SIZE: usize = 4 * 1024 * 1024;

#[tokio::test]
async fn a_moved_download_resumes_from_its_new_place() {
    let server = TestServer::http();
    let data = random_bytes(SIZE, 71);
    server.put(
        "/f.bin",
        FileSpec::new(data.clone()).cut_after(SIZE as u64 / 3, 1),
    );
    let tmp = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tmp.path()).unwrap();
    let (first, chosen) = (root.join("first"), root.join("chosen"));
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&chosen).unwrap();
    let engine = Engine::new(Options {
        connections: 1,
        max_failures: 1,
        ..Options::default()
    })
    .unwrap();
    let state_file = match engine
        .download(&server.url("/f.bin"), &first, &mut |_| {})
        .await
    {
        Err(Error::Interrupted { state_file, .. }) => state_file,
        other => panic!("expected an interruption, got {other:?}"),
    };
    let on_disk = DownloadState::load(&state_file).unwrap().bytes_done();
    assert!(on_disk > 0);

    let moved = tokio::task::spawn_blocking({
        let (state_file, chosen) = (state_file.clone(), chosen.clone());
        move || relocate::move_unfinished(&state_file, &chosen, "الملف.bin")
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(fs::read_dir(&first).unwrap().count(), 0);

    let done = engine.resume(&moved, &mut |_| {}).await.unwrap();
    assert_eq!(done.file, chosen.join("الملف.bin"));
    assert_eq!(
        sha256_hex(&fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );
    // Only the missing bytes were asked for again.
    let gets = server.gets("/f.bin");
    assert_eq!(gets.len(), 2);
    assert_eq!(gets[1].range, Some(format!("bytes={on_disk}-")));
}
