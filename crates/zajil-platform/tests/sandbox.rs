//! The sandbox really holds: a program in it writes in its folder, but not
//! outside, starts no other program, and reaches no TCP port where the
//! system refuses the network.
//!
//! [`Sandbox::enter`] confines the calling process for good, so the test
//! runs this same test binary twice more: once as the confiner (it enters
//! the sandbox), which then starts it again as the probe (it tries each
//! thing and says what worked). The test itself is never confined. No
//! test harness (`harness = false` in Cargo.toml): the confiner must have
//! one thread only, and libtest runs each test on a thread of its own.

use std::io::Write;
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;

use zajil_platform::sandbox::Sandbox;

const ROLE: &str = "ZAJIL_SANDBOX_TEST_ROLE";

/// Runs this test binary as `role`.
fn this_binary(role: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.env(ROLE, role);
    command
}

/// What the probe tries; its exit code says whether it worked.
fn try_it(action: &str, target: &std::ffi::OsStr) -> bool {
    match action {
        "write" => std::fs::File::create(target)
            .and_then(|mut f| f.write_all(b"x"))
            .is_ok(),
        // Any other program: a shell that does nothing.
        "exec" => {
            let mut other = if cfg!(windows) {
                let mut c = std::process::Command::new(
                    std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into()),
                );
                c.args(["/c", "exit 0"]);
                c
            } else {
                let mut c = std::process::Command::new("/bin/sh");
                c.args(["-c", "true"]);
                c
            };
            other.status().is_ok_and(|s| s.success())
        }
        "connect" => {
            let port: u16 = std::env::var("ZAJIL_PORT").unwrap().parse().unwrap();
            std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
        }
        other => panic!("unknown action {other}"),
    }
}

fn probe() {
    let action = std::env::var("ZAJIL_ACTION").unwrap();
    let target = std::env::var_os("ZAJIL_TARGET").unwrap();
    std::process::exit(if try_it(&action, &target) { 0 } else { 3 });
}

fn confiner() {
    let work = std::env::var_os("ZAJIL_WORK").unwrap();
    let me = std::env::current_exe().unwrap();
    let sandbox = Sandbox::enter(Path::new(&work), &me).unwrap();
    let confinement = sandbox.confinement();
    let run = |action: &str, target: &std::ffi::OsStr| {
        // On Linux the confiner is in the sandbox itself, and a dynamic
        // program (this test binary) cannot be started in it: running its
        // loader would let anything run (`ld.so /bin/sh`). It tries each
        // thing itself. Elsewhere only the started program is confined.
        if cfg!(target_os = "linux") {
            return if try_it(action, target) { "yes" } else { "no" };
        }
        let mut command = sandbox.command();
        command
            .env(ROLE, "probe")
            .env("ZAJIL_ACTION", action)
            .env("ZAJIL_TARGET", target)
            .stdout(std::process::Stdio::null());
        let mut confined = sandbox.spawn(command).unwrap();
        let status = confined.child().wait().unwrap();
        match status.code() {
            Some(0) => "yes",
            Some(3) => "no",
            _ => panic!("the probe failed: {status}"),
        }
    };
    let inside = Path::new(&work).join("inside.txt");
    let outside = std::env::var_os("ZAJIL_OUTSIDE").unwrap();
    println!(
        "zajil-result programs={} files={} network={} inside={} outside={} connect={} exec={}",
        confinement.programs,
        confinement.files,
        confinement.network,
        run("write", inside.as_os_str()),
        run("write", &outside),
        run("connect", "".as_ref()),
        run("exec", "".as_ref()),
    );
}

fn a_sandboxed_program_writes_only_in_its_folder_starts_nothing_and_reaches_no_port() {
    let work = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let outside = elsewhere.path().join("outside.txt");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let output = this_binary("confiner")
        .env("ZAJIL_WORK", work.path())
        .env("ZAJIL_OUTSIDE", &outside)
        .env("ZAJIL_PORT", port.to_string())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let line = text
        .lines()
        .find_map(|l| l.split_once("zajil-result ").map(|(_, rest)| rest))
        .unwrap_or_else(|| panic!("no result: {text}"));
    let field = |name: &str| {
        line.split_whitespace()
            .find_map(|w| w.strip_prefix(&format!("{name}=")))
            .unwrap()
            .to_owned()
    };
    // In its folder it always writes: the sandbox must not stop it working.
    assert_eq!(field("inside"), "yes", "{line}");
    assert!(work.path().join("inside.txt").exists());
    let on_ci = std::env::var_os("CI").is_some();
    if field("files") == "true" {
        assert_eq!(field("outside"), "no", "{line}");
        assert!(!outside.exists());
    } else {
        // A system without it (an old Linux) still runs the program; on CI
        // every system must have it.
        assert!(!on_ci, "no file sandbox on CI: {line}");
        eprintln!("NOT RUN: this system has no file sandbox ({line})");
    }
    if field("programs") == "true" {
        assert_eq!(field("exec"), "no", "{line}");
    } else {
        assert!(!on_ci, "no program ban on CI: {line}");
        eprintln!("NOT RUN: this system cannot refuse other programs ({line})");
    }
    if field("network") == "true" {
        assert_eq!(field("connect"), "no", "{line}");
    } else {
        // Windows never refuses the network (low integrity does not); an
        // old Linux may not.
        eprintln!("this system does not refuse the network to the sandbox ({line})");
    }
    drop(listener);
}

fn main() {
    match std::env::var(ROLE).as_deref() {
        Ok("probe") => probe(),
        Ok("confiner") => confiner(),
        _ => {
            a_sandboxed_program_writes_only_in_its_folder_starts_nothing_and_reaches_no_port();
            println!("sandbox: ok");
        }
    }
}
