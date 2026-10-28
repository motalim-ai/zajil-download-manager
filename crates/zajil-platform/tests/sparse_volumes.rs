//! `allocate` reserves nothing on the file systems downloads land on
//! (owner's decision, 2026-10-28): a file of 1 GiB fits on a volume of a few
//! hundred MiB, and pieces written far apart take only their own blocks.
//!
//! CI mounts a small image of each file system of the runner's system and
//! passes its folder: `ZAJIL_TEST_EXT4` and `ZAJIL_TEST_BTRFS` on Linux,
//! `ZAJIL_TEST_APFS` on macOS, `ZAJIL_TEST_NTFS` on Windows (see
//! `.github/workflows/ci.yml`). On CI a missing variable is a failure, so
//! the test cannot silently stop testing anything; elsewhere it is skipped
//! with a message.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use zajil_platform::fs::{allocate, available_space, size_on_disk, write_at};

const GIB: u64 = 1 << 30;
const MIB: u64 = 1 << 20;

/// `(name, folder)` for each mounted test volume of this system.
fn volumes() -> Vec<(&'static str, PathBuf)> {
    let mut found = Vec::new();
    for (name, var, os) in [
        ("ext4", "ZAJIL_TEST_EXT4", "linux"),
        ("btrfs", "ZAJIL_TEST_BTRFS", "linux"),
        ("APFS", "ZAJIL_TEST_APFS", "macos"),
        ("NTFS", "ZAJIL_TEST_NTFS", "windows"),
    ] {
        if std::env::consts::OS != os {
            continue;
        }
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
fn a_file_larger_than_the_volume_takes_only_the_blocks_written() {
    const PIECE: usize = 64 << 10;
    for (name, volume) in volumes() {
        let dir = workdir(&volume, "sparse");
        let free = available_space(&volume).unwrap_or_else(|e| panic!("{name}: {e}"));
        // The images are smaller than the file.
        assert!(free < GIB, "{name}: {free} bytes free");

        let path = dir.join("f.part");
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        allocate(&file, GIB).unwrap_or_else(|e| panic!("{name}: {e}"));
        file.sync_all().unwrap();
        let empty = size_on_disk(&path).unwrap();
        let free_after = available_space(&volume).unwrap();
        eprintln!(
            "{name}: 1 GiB after allocate takes {empty} bytes; free {free} -> {free_after} bytes"
        );
        assert_eq!(file.metadata().unwrap().len(), GIB, "{name}");
        assert!(empty < MIB, "{name}: {empty} bytes reserved");
        assert!(
            free_after + 4 * MIB > free,
            "{name}: {free} -> {free_after}"
        );

        // As the connections of a split download write: the gaps between
        // the pieces stay holes, never zeros written to the disk.
        let offsets = [0, GIB / 4, GIB / 2, GIB - PIECE as u64];
        for (i, &at) in offsets.iter().enumerate() {
            write_at(&file, &[i as u8 + 1; PIECE], at).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        file.sync_all().unwrap();
        drop(file);
        let used = size_on_disk(&path).unwrap();
        eprintln!("{name}: after 4 pieces of 64 KiB: {used} bytes on disk");
        assert!(
            used >= 4 * PIECE as u64 && used < 4 * MIB,
            "{name}: {used} bytes"
        );

        let mut file = fs::File::open(&path).unwrap();
        let mut piece = vec![0; PIECE];
        for (i, &at) in offsets.iter().enumerate() {
            file.seek(SeekFrom::Start(at)).unwrap();
            file.read_exact(&mut piece).unwrap();
            assert!(piece.iter().all(|&b| b == i as u8 + 1), "{name}: {i}");
        }
        file.seek(SeekFrom::Start(GIB / 8)).unwrap();
        file.read_exact(&mut piece).unwrap();
        assert!(piece.iter().all(|&b| b == 0), "{name}: the gap");
        drop(file);
        fs::remove_dir_all(&dir).unwrap();
    }
}
