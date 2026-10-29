//! A disk that fills up in the middle of a download. Nothing is reserved
//! when a download starts, on any system (owner's decision, 2026-10-28), so
//! the disk can fill up later: the download stops at once (a full disk does
//! not pass by trying again), keeps what is on disk, and resumes once there
//! is room again.
//!
//! The disk is a small volume of the system's own file system, which CI
//! mounts and passes in `ZAJIL_TEST_EXT4` (Linux), `ZAJIL_TEST_APFS` (macOS)
//! or `ZAJIL_TEST_NTFS` (Windows); see `.github/workflows/ci.yml`. On CI a
//! missing variable is a failure; elsewhere the test is skipped with a
//! message.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use zajil_core::state::{DownloadState, part_path};
use zajil_core::{Engine, Error, Event, Options};
use zajil_platform::fs::{available_space, size_on_disk};
use zajil_testserver::{FileSpec, RequestRecord, TestServer, random_bytes, sha256_hex};

const MIB: usize = 1 << 20;
const SIZE: usize = 16 * MIB;

/// The mounted small volume of this system's file system, if any.
fn volume() -> Option<(&'static str, PathBuf)> {
    let (name, var) = match std::env::consts::OS {
        "linux" => ("ext4", "ZAJIL_TEST_EXT4"),
        "macos" => ("APFS", "ZAJIL_TEST_APFS"),
        "windows" => ("NTFS", "ZAJIL_TEST_NTFS"),
        other => {
            eprintln!("skipping: no test volume for {other}");
            return None;
        }
    };
    match std::env::var_os(var) {
        Some(dir) => Some((name, PathBuf::from(dir))),
        None if std::env::var_os("CI").is_some() => {
            panic!("CI must mount a {name} image and set {var}")
        }
        None => {
            eprintln!("skipping {name}: {var} is not set");
            None
        }
    }
}

/// A file that takes all the free space of a volume, with zeros written for
/// real (no holes). Removed when dropped, even when the test fails.
///
/// On CI's NTFS a filler closed at once left room: a download of 12 MiB
/// more finished on the "full" volume, and only its last save failed. A
/// file system may give a growing file more blocks than it wrote and free
/// the rest when it is closed, so the filler stays open until dropped, and
/// filling goes on until a new file cannot take a block either.
///
/// APFS frees a removed file's blocks in the background: on CI a download
/// resumed right after the filler went still found the disk full. So the
/// drop waits, up to 30 s, until the room is back.
struct Filler {
    path: PathBuf,
    file: Option<fs::File>,
    volume: PathBuf,
    free_before: u64,
}

impl Filler {
    fn fill(volume: &Path) -> Self {
        let path = volume.join(format!("filler-{}", std::process::id()));
        let probe = volume.join(format!("probe-{}", std::process::id()));
        let free_before = available_space(volume).unwrap();
        let mut file = fs::File::create(&path).unwrap();
        for _ in 0..64 {
            for chunk in [MIB, 4096] {
                let zeros = vec![0; chunk];
                while file.write_all(&zeros).is_ok() {}
            }
            let _ = file.sync_all();
            let room = fs::File::create(&probe)
                .and_then(|mut f| f.write_all(&[0; 4096]).and_then(|()| f.sync_all()));
            let _ = fs::remove_file(&probe);
            if room.is_err() {
                return Filler {
                    path,
                    file: Some(file),
                    volume: volume.to_path_buf(),
                    free_before,
                };
            }
        }
        panic!("{} could not be filled", volume.display());
    }
}

impl Drop for Filler {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = fs::remove_file(&self.path);
        // Enough to finish a download, or all there was before.
        let wanted = self.free_before.min(2 * SIZE as u64);
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while available_space(&self.volume).is_ok_and(|free| free < wanted)
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// The `(from, to)` of a `Range: bytes=from-to` or `bytes=from-` request.
fn range_of(record: &RequestRecord) -> Option<(u64, u64)> {
    let spec = record.range.as_deref()?.strip_prefix("bytes=")?;
    let (from, to) = spec.split_once('-')?;
    let to = if to.is_empty() {
        SIZE as u64 - 1
    } else {
        to.parse().ok()?
    };
    Some((from.parse().ok()?, to))
}

/// One connection, then four: the disk fills up a quarter of the way in,
/// once a save has recorded at least 1 MiB (a fixed point in time did not
/// do: on CI's macOS runner the last save before it had recorded only
/// 786432 bytes).
#[tokio::test]
async fn a_full_disk_stops_the_download_which_resumes_once_there_is_room() {
    let Some((name, volume)) = volume() else {
        return;
    };
    let server = TestServer::http();
    let data = random_bytes(SIZE, 91);
    for connections in [1, 4] {
        let case = format!("{name}, {connections} connection(s)");
        let path = format!("/f{connections}.bin");
        // 4 MiB/s in all: about 40 periodic saves (every 100 ms) can land
        // before the end.
        let spec = FileSpec::new(data.clone()).throttle(4 * MIB as u64 / connections as u64);
        server.put(&path, spec);
        let dir = volume.join(format!("core-full-{connections}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        let engine = Engine::new(Options {
            connections,
            save_interval: Duration::from_millis(100),
            ..Options::default()
        })
        .unwrap();

        let free_before = available_space(&volume).unwrap();
        let mut at_start = None;
        let mut state_file = None;
        // What the state file on disk counted when the disk was filled.
        let mut saved_before_full = None;
        let mut filler = None;
        let result = engine
            .download(&server.url(&path), &dir, &mut |event| match event {
                Event::Started {
                    file,
                    state_file: saved,
                    ..
                } => {
                    state_file = Some(saved);
                    // The part file has its length, and takes nothing.
                    let part = part_path(&file);
                    at_start = Some((
                        fs::metadata(&part).unwrap().len(),
                        size_on_disk(&part).unwrap(),
                        available_space(&volume).unwrap(),
                    ));
                }
                // Filled here, blocking on purpose: the test's runtime has
                // one thread, so no connection writes or saves meanwhile.
                Event::Progress { done, .. } if filler.is_none() && done >= SIZE as u64 / 4 => {
                    let saved = state_file
                        .as_deref()
                        .and_then(|path| DownloadState::load(path).ok())
                        .map_or(0, |state| state.bytes_done());
                    if saved >= MIB as u64 {
                        saved_before_full = Some(saved);
                        filler = Some(Filler::fill(&volume));
                    }
                }
                _ => {}
            })
            .await;

        let (len, on_disk, free_at_start) = at_start.expect("the download started");
        eprintln!(
            "{case}: at the start the part file is {len} bytes long and takes {on_disk}; \
             free {free_before} -> {free_at_start}"
        );
        assert_eq!(len, SIZE as u64, "{case}");
        assert!(on_disk < MIB as u64, "{case}: {on_disk} bytes reserved");
        // A second download of the same size still fits (the owner's
        // report: with the space reserved, it failed at once).
        assert!(free_at_start + MIB as u64 > free_before, "{case}");
        assert!(free_at_start > 2 * SIZE as u64, "{case}");

        let filler = filler.expect("no save recorded 1 MiB before the end");
        let saved_before_full = saved_before_full.expect("set with the filler");
        let (state_file, reason) = match result {
            Err(Error::Interrupted { state_file, reason }) => (state_file, reason),
            other => panic!("{case}: expected an interruption, got {other:?}"),
        };
        eprintln!("{case}: stopped: {reason}");
        // The disk's own error, not a server's.
        assert!(reason.contains("os error"), "{case}: {reason}");
        // Stopped at once: no connection asked again.
        let first = server.gets(&path);
        assert!(
            first.len() <= connections,
            "{case}: {} requests",
            first.len()
        );

        // The state file is whole (its checksum holds), and counts at least
        // what it did when the disk filled up: a save that failed on the
        // full disk never touched it, nor left its temporary file.
        let state = DownloadState::load(&state_file).unwrap();
        let kept = state.bytes_done();
        eprintln!("{case}: {kept} bytes kept of {SIZE}, {saved_before_full} saved before");
        assert!(
            kept >= saved_before_full,
            "{case}: {kept} bytes kept, {saved_before_full} were saved"
        );
        let tmp = state_file.with_file_name(format!(
            "{}.tmp",
            state_file.file_name().unwrap().to_str().unwrap()
        ));
        assert!(!tmp.exists(), "{case}: {} left", tmp.display());
        // What the state file counts is on disk.
        let part = fs::read(part_path(&dir.join(format!("f{connections}.bin")))).unwrap();
        for segment in &state.segments {
            let (start, end) = (
                segment.start as usize,
                (segment.start + segment.done) as usize,
            );
            assert!(part[start..end] == data[start..end], "{case}: {segment:?}");
        }

        // Room again: the download resumes from what it kept.
        drop(filler);
        eprintln!(
            "{case}: free once the filler went: {}",
            available_space(&volume).unwrap()
        );
        let done = engine
            .resume(&state_file, &mut |_| {})
            .await
            .unwrap_or_else(|e| panic!("{case}: resuming: {e}"));
        assert_eq!(
            sha256_hex(&fs::read(&done.file).unwrap()),
            sha256_hex(&data)
        );
        let again = &server.gets(&path)[first.len()..];
        assert!(!again.is_empty(), "{case}");
        for request in again {
            let (from, to) = range_of(request).unwrap_or_else(|| panic!("{case}: {request:?}"));
            for segment in &state.segments {
                let kept_end = segment.start + segment.done;
                assert!(
                    to < segment.start || from >= kept_end,
                    "{case}: {from}-{to} asks again for kept bytes of {segment:?}"
                );
            }
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
