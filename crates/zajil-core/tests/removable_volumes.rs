//! Downloads finish on FAT32 and exFAT (USB drives). Before M2 no download
//! could finish on exFAT under macOS: the volume refuses both the no-replace
//! rename and hard links (#4).
//!
//! CI mounts a small image of each and passes its folder in
//! `ZAJIL_TEST_FAT32` and `ZAJIL_TEST_EXFAT`. On CI a missing variable is a
//! failure; elsewhere the test is skipped with a message.

use std::fs;
use std::path::PathBuf;

use zajil_core::{Engine, Options};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

fn volumes() -> Vec<(&'static str, PathBuf)> {
    let mut found = Vec::new();
    for (name, var) in [("FAT32", "ZAJIL_TEST_FAT32"), ("exFAT", "ZAJIL_TEST_EXFAT")] {
        match std::env::var_os(var) {
            Some(dir) => found.push((name, PathBuf::from(dir))),
            None if std::env::var_os("CI").is_some() => {
                panic!("CI must mount a {name} image and set {var}")
            }
            None => eprintln!("skipping {name}: {var} is not set"),
        }
    }
    found
}

#[tokio::test]
async fn downloads_finish_next_to_a_taken_name() {
    let server = TestServer::http();
    // 4 MiB: big enough to be split over several connections (M2).
    let data = random_bytes(4 << 20, 7);
    server.put("/f.bin", FileSpec::new(data.clone()));
    let engine = Engine::new(Options::default()).unwrap();

    for (name, volume) in volumes() {
        let dir = volume.join(format!("core-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();

        let first = engine
            .download(&server.url("/f.bin"), &dir, &mut |_| {})
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(first.file, dir.join("f.bin"), "{name}");
        // The name is taken now: the second one is numbered, never written
        // over the first.
        let second = engine
            .download(&server.url("/f.bin"), &dir, &mut |_| {})
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(second.file, dir.join("f (1).bin"), "{name}");
        for file in [&first.file, &second.file] {
            assert_eq!(
                sha256_hex(&fs::read(file).unwrap()),
                sha256_hex(&data),
                "{name}"
            );
        }
        let mut left: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            // macOS keeps extended attributes of files on FAT and exFAT in
            // `._<name>` files of its own.
            .filter(|n| !n.starts_with("._"))
            .collect();
        left.sort();
        assert_eq!(left, ["f (1).bin", "f.bin"], "{name}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
