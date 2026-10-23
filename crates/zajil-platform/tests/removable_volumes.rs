//! The file helpers on FAT32 and exFAT, the file systems of USB drives.
//!
//! CI mounts a small disk image of each and passes its folder in
//! `ZAJIL_TEST_FAT32` and `ZAJIL_TEST_EXFAT` (see `.github/workflows/ci.yml`).
//! On CI a missing variable is a failure, so the test cannot silently stop
//! testing anything; elsewhere it is skipped with a message.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use zajil_platform::fs::{allocate, available_space, rename_noreplace, write_at};

/// `(name, folder)` for each mounted test volume.
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

/// A fresh folder on the volume, so runs and tests do not meet.
fn workdir(volume: &Path, test: &str) -> PathBuf {
    let dir = volume.join(format!("{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).unwrap();
    dir
}

#[test]
fn rename_noreplace_works_and_never_overwrites() {
    for (name, volume) in volumes() {
        let dir = workdir(&volume, "rename");
        let (part, taken, free) = (dir.join("f.part"), dir.join("f.bin"), dir.join("g.bin"));
        fs::write(&part, b"new").unwrap();
        fs::write(&taken, b"old").unwrap();

        let err = rename_noreplace(&part, &taken).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{name}: {err}");
        assert_eq!(fs::read(&taken).unwrap(), b"old", "{name}");
        assert_eq!(fs::read(&part).unwrap(), b"new", "{name}");

        rename_noreplace(&part, &free).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(fs::read(&free).unwrap(), b"new", "{name}");
        assert!(!part.exists(), "{name}");
        fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn allocate_and_write_at_work() {
    for (name, volume) in volumes() {
        let dir = workdir(&volume, "write");
        let path = dir.join("f.part");
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        allocate(&file, 1 << 20).unwrap_or_else(|e| panic!("{name}: {e}"));
        write_at(&file, b"end", (1 << 20) - 3).unwrap();
        write_at(&file, b"start", 0).unwrap();
        drop(file);
        let data = fs::read(&path).unwrap();
        assert_eq!(data.len(), 1 << 20, "{name}");
        assert!(
            data.starts_with(b"start") && data.ends_with(b"end"),
            "{name}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn available_space_is_the_small_volume_and_allocate_beyond_it_fails() {
    for (name, volume) in volumes() {
        let free = available_space(&volume).unwrap_or_else(|e| panic!("{name}: {e}"));
        // The images are 256 MiB.
        assert!(free > 0 && free <= 256 << 20, "{name}: {free}");

        // More than the volume holds (and, on FAT32, more than 4 GiB): an
        // error from the system, never a panic, and the file stays empty.
        let dir = workdir(&volume, "too-big");
        let file = fs::File::create(dir.join("f.part")).unwrap();
        let err = allocate(&file, 5 << 30).unwrap_err();
        eprintln!("{name}: allocating 5 GiB: {err} ({:?})", err.kind());
        drop(file);
        fs::remove_dir_all(&dir).unwrap();
    }
}
