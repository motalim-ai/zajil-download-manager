//! A tool's process tree ends with it, and with Zajil (ADR-0009, section 4).
//!
//! This test program plays every part, chosen by `ZAJIL_PROCESS_ROLE`:
//! - `beat`: a process that appends to a file every 20 ms, forever (what a
//!   leftover yt-dlp or ffmpeg would be);
//! - `tool`: starts a `beat` grandchild, then waits forever, or exits at
//!   once with `ZAJIL_TOOL_EXIT=1`;
//! - `zajil`: starts a `tool` as a [`Tree`], writes the tree's [`Record`],
//!   then waits to be killed (Zajil crashing mid-download).
//!
//! A process has ended once its file stops growing.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use zajil_platform::process::{self, Record};

const ROLE: &str = "ZAJIL_PROCESS_ROLE";
const BEAT_FILE: &str = "ZAJIL_BEAT_FILE";

/// This test program, running only the `role` entry below.
fn me(role: &str, beat: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["role_entry", "--exact", "--nocapture", "--test-threads=1"])
        .env(ROLE, role)
        .env(BEAT_FILE, beat)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// The entry of the helper roles; does nothing in a normal test run.
#[test]
fn role_entry() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    let beat = PathBuf::from(std::env::var(BEAT_FILE).unwrap());
    match role.as_str() {
        "beat" => loop {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&beat)
                .unwrap();
            file.write_all(b".").unwrap();
            drop(file);
            std::thread::sleep(Duration::from_millis(20));
        },
        "tool" => {
            // Left running on purpose: the grandchild the tree must end.
            #[allow(clippy::zombie_processes)]
            me("beat", &beat).spawn().unwrap();
            if std::env::var("ZAJIL_TOOL_EXIT").is_ok() {
                // Leaves its child behind, like a loader that ends first.
                std::process::exit(0);
            }
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        }
        "zajil" => {
            let tree = process::spawn(me("tool", &beat)).unwrap();
            let record = tree.record();
            std::fs::write(beat.with_extension("record"), record.to_line()).unwrap();
            std::mem::forget(tree);
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        }
        other => panic!("unknown role {other}"),
    }
}

fn size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Waits until `path` grows (the beat is running), at most 20 s.
fn wait_beating(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let start = size(path);
    while size(path) <= start {
        assert!(Instant::now() < deadline, "the beat never started");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Whether the beat at `path` stopped: it does not grow over 500 ms, 25
/// times its period.
fn stopped(path: &Path) -> bool {
    // Let a write in progress land first.
    std::thread::sleep(Duration::from_millis(100));
    let before = size(path);
    std::thread::sleep(Duration::from_millis(500));
    size(path) == before
}

#[test]
fn killing_a_tree_ends_its_grandchildren() {
    let dir = tempfile::tempdir().unwrap();
    let beat = dir.path().join("beat");
    let mut tree = process::spawn(me("tool", &beat)).unwrap();
    wait_beating(&beat);
    tree.kill().unwrap();
    assert!(stopped(&beat), "the grandchild still runs");
}

#[test]
fn a_killer_ends_the_tree_from_another_thread() {
    let dir = tempfile::tempdir().unwrap();
    let beat = dir.path().join("beat");
    let mut tree = process::spawn(me("tool", &beat)).unwrap();
    wait_beating(&beat);
    let killer = tree.killer();
    let waiter = std::thread::spawn(move || tree.wait().unwrap());
    killer.kill().unwrap();
    let status = waiter.join().unwrap();
    assert!(!status.success(), "{status:?}");
    assert!(stopped(&beat), "the grandchild still runs");
    // Again, once it is gone: nothing to do, no error.
    killer.kill().unwrap();
}

#[test]
fn dropping_a_tree_ends_it() {
    let dir = tempfile::tempdir().unwrap();
    let beat = dir.path().join("beat");
    let tree = process::spawn(me("tool", &beat)).unwrap();
    wait_beating(&beat);
    drop(tree);
    assert!(stopped(&beat), "the grandchild still runs");
}

#[test]
fn waiting_ends_what_the_first_process_left_behind() {
    let dir = tempfile::tempdir().unwrap();
    let beat = dir.path().join("beat");
    let mut command = me("tool", &beat);
    command.env("ZAJIL_TOOL_EXIT", "1");
    let mut tree = process::spawn(command).unwrap();
    wait_beating(&beat);
    let status = tree.wait().unwrap();
    assert!(status.success(), "{status:?}");
    // wait() returned, so nothing of the tree may write any more.
    assert!(stopped(&beat), "the grandchild still runs after wait()");
}

#[test]
fn a_tree_left_by_a_crashed_zajil_is_ended() {
    let dir = tempfile::tempdir().unwrap();
    let beat = dir.path().join("beat");
    // A Zajil that started a tool, then is killed without any cleanup. Its
    // first process may then be gone and its id reused: only the token in
    // the environment of what is left identifies the tree.
    let mut zajil = me("zajil", &beat).spawn().unwrap();
    wait_beating(&beat);
    let record =
        Record::from_line(&std::fs::read_to_string(beat.with_extension("record")).unwrap())
            .expect("the record");
    zajil.kill().unwrap();
    zajil.wait().unwrap();
    if cfg!(windows) {
        // The job's last handle closed with Zajil: the tree ended with it.
        assert!(stopped(&beat), "the job did not end the tree with Zajil");
    }
    // The next start of Zajil ends whatever is left (on macOS and Linux:
    // the death signal reaches only the first process).
    process::end_leftover(record).unwrap();
    assert!(stopped(&beat), "the tree outlived Zajil and its next start");
}

#[test]
fn another_run_s_record_leaves_this_tree_alone() {
    let dir = tempfile::tempdir().unwrap();
    let beat = dir.path().join("beat");
    let mut tree = process::spawn(me("tool", &beat)).unwrap();
    wait_beating(&beat);
    // Another run's token: nothing of this tree carries it.
    let other = process::spawn(me("tool", &dir.path().join("other"))).unwrap();
    let other_record = other.record();
    drop(other);
    assert_ne!(other_record, tree.record());
    process::end_leftover(other_record).unwrap();
    assert!(!stopped(&beat), "a tree with another token was ended");
    tree.kill().unwrap();
}

#[cfg(unix)]
#[test]
fn files_a_tool_creates_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("made");
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "echo x > \"$1\"", "sh"]).arg(&file);
    let mut tree = process::spawn(command).unwrap();
    assert!(tree.wait().unwrap().success());
    let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "umask 077 was not applied: {mode:o}");
}
