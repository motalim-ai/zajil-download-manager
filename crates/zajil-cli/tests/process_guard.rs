//! A `zajil` that a test started in the background never outlives the
//! test, even when the test fails before stopping it (#50).

mod common;

use std::process::Stdio;
use std::time::{Duration, Instant};

use zajil_testserver::{FileSpec, TestServer, random_bytes};

/// Whether a process with this id still runs.
fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    let out = std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success());
    #[cfg(windows)]
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()));
    out.unwrap()
}

#[test]
fn a_failed_test_does_not_leave_zajil_running() {
    let server = TestServer::http();
    // A download that never ends: the server stops sending after 1 KiB.
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(1024 * 1024, 7)).stall_after(1024),
    );
    let dir = tempfile::tempdir().unwrap();
    let url = server.url("/f.bin");
    let mut pid = 0;
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let child = common::zajil_command()
            .args(["get", "--dir"])
            .arg(dir.path())
            .arg(&url)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let child = common::Running(child);
        pid = child.id();
        std::thread::sleep(Duration::from_millis(500));
        assert!(alive(pid), "zajil should still be downloading");
        panic!("the test fails here, before stopping zajil");
    }));
    assert!(failed.is_err());
    // The guard killed it and waited for it.
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive(pid) {
        assert!(
            Instant::now() < deadline,
            "zajil {pid} still runs after the test failed"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Every `zajil` a test starts in the background is held by the guard:
/// as many `.spawn()` as `common::Running(` in each test file.
#[test]
fn every_background_zajil_is_held_by_the_guard() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut checked = 0;
    for entry in std::fs::read_dir(&folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let spawned = text.matches(".spawn()").count();
        let held = text.matches("common::Running(").count();
        assert_eq!(
            spawned,
            held,
            "{}: {spawned} started, {held} held",
            path.display()
        );
        checked += spawned;
    }
    assert!(checked >= 7, "found only {checked} background processes");
}
