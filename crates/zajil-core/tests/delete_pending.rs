//! #55: cancelling a new download, then adding it again at once, while
//! another program holds the cancelled download's files open (an antivirus
//! or the search indexer, which share the files' deletion).
//!
//! On Windows a file removed while a handle on it is open stays "delete
//! pending": its name stays taken until the last handle closes, and
//! creating a file of that name fails with `ERROR_ACCESS_DENIED`. The
//! engine removed the cancelled download's files in place, so the new
//! download failed with "Access is denied. (os error 5)". Now the files are
//! moved aside first, which frees their names at once.
//!
//! No timing: the "antivirus" handle is opened as soon as the part file
//! exists (`Event::Started`), and `download_until` returns only once the
//! cancelled download's files are dealt with.
//!
//! Run in the system's temporary folder and on the FAT32 and exFAT volumes
//! CI mounts (`ZAJIL_TEST_FAT32`, `ZAJIL_TEST_EXFAT`, as in
//! `removable_volumes.rs`). On NTFS, Windows removes a file POSIX-style,
//! which frees its name at once even while another program holds it; FAT32
//! and exFAT (USB drives) have no such removal, so there a held file stays
//! "delete pending" throughout: the case this test fails without the fix
//! (shown on windows-latest, #55). On macOS and Linux a removed file never
//! keeps its name, so there the test passes with or without the fix: it
//! guards Windows, and checks the rest still works everywhere.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::Duration;

use zajil_core::{Engine, Error, Event, Options, Stop};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

fn engine() -> Engine {
    Engine::new(Options {
        idle_timeout: Duration::from_secs(5),
        ..Options::default()
    })
    .unwrap()
}

/// The folders to test in: the system's temporary folder, then the FAT32
/// and exFAT volumes (on CI a missing one is a failure).
fn folders() -> Vec<(&'static str, Option<PathBuf>)> {
    let mut found = vec![("temporary folder", None)];
    for (name, var) in [("FAT32", "ZAJIL_TEST_FAT32"), ("exFAT", "ZAJIL_TEST_EXFAT")] {
        match std::env::var_os(var) {
            Some(dir) => found.push((name, Some(PathBuf::from(dir)))),
            None if std::env::var_os("CI").is_some() => {
                panic!("CI must mount a {name} image and set {var}")
            }
            None => eprintln!("skipping {name}: {var} is not set"),
        }
    }
    found
}

#[tokio::test]
async fn a_cancelled_download_added_again_gets_its_name_while_an_antivirus_holds_its_files() {
    let server = TestServer::http();
    let data = random_bytes(300_000, 55);
    for (name, volume) in folders() {
        let dir = match volume {
            Some(volume) => tempfile::tempdir_in(volume).unwrap(),
            None => tempfile::tempdir().unwrap(),
        };
        cancel_then_add_again(&server, &data, dir.path(), name).await;
    }
}

async fn cancel_then_add_again(server: &TestServer, data: &[u8], dir: &Path, name: &str) {
    // Headers, then nothing: the download waits for its first byte.
    server.put("/f.bin", FileSpec::new(data.to_vec()).stall_after(0));
    let stop = Stop::new();
    let mut antivirus: Option<File> = None;
    let cancelled = engine()
        .download_until(&server.url("/f.bin"), dir, &stop, &mut |event| {
            if let Event::Started { file, .. } = event
                && antivirus.is_none()
            {
                // Opened as an antivirus opens it: reading, sharing
                // reading, writing and deletion (Rust's default share
                // mode on Windows).
                let part = dir.join(format!(
                    "{}.zajil-part",
                    file.file_name().unwrap().to_string_lossy()
                ));
                antivirus = Some(File::open(&part).expect("the part file exists"));
                stop.request();
            }
        })
        .await;
    assert!(
        matches!(cancelled, Err(Error::Stopped { state_file: None })),
        "{name}: {cancelled:?}"
    );
    assert!(antivirus.is_some(), "{name}: the download never started");

    // Added again at once, while the "antivirus" still holds the file.
    server.put("/f.bin", FileSpec::new(data.to_vec()));
    let done = engine()
        .download(&server.url("/f.bin"), dir, &mut |_| {})
        .await
        .unwrap_or_else(|e| panic!("{name}: the new download of the same name: {e}"));
    assert_eq!(
        done.file.file_name().unwrap(),
        "f.bin",
        "{name}: the same name, not a numbered one"
    );
    assert_eq!(
        sha256_hex(&fs::read(&done.file).unwrap()),
        sha256_hex(data),
        "{name}"
    );

    // Once the "antivirus" lets go, nothing of the cancelled download is left.
    drop(antivirus);
    let left: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        // macOS keeps extended attributes in `._<name>` files on FAT and
        // exFAT (as in removable_volumes.rs; review of #56).
        .filter(|n| !n.starts_with("._"))
        .collect();
    assert_eq!(left, ["f.bin"], "{name}");
}
