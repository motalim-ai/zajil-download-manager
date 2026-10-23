//! End-to-end tests of the `zajil` binary against the local test server.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use zajil_core::state::DownloadState;
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

fn zajil(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_zajil"))
        .args(args)
        .output()
        .expect("run zajil")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn only_state_file(dir: &Path) -> PathBuf {
    let states: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".zajil-state"))
        .collect();
    assert_eq!(states.len(), 1, "state files: {states:?}");
    states.into_iter().next().unwrap()
}

#[test]
fn get_downloads_into_the_chosen_folder() {
    let server = TestServer::http();
    let data = random_bytes(3_000_000, 1);
    server.put("/file.bin", FileSpec::new(data.clone()));
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("new").join("folder");

    let out = zajil(&[
        "get",
        &server.url("/file.bin"),
        "--dir",
        target.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let saved = target.join("file.bin");
    assert_eq!(sha256_hex(&fs::read(&saved).unwrap()), sha256_hex(&data));
    assert!(text(&out.stdout).contains("Saved"));
    assert!(text(&out.stdout).contains("file.bin"));
}

/// The resume commands `zajil` printed, by shell: lines like
/// `zajil: to resume, run this in <shell>:` followed by the indented
/// command.
fn printed_commands(stderr: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut lines = stderr.lines();
    while let Some(line) = lines.next() {
        if let Some(shell) = line
            .strip_prefix("zajil: to resume, run this in ")
            .and_then(|rest| rest.strip_suffix(':'))
        {
            let command = lines.next().expect("command line").trim_start();
            found.push((shell.to_owned(), command.to_owned()));
        }
    }
    assert!(!found.is_empty(), "no resume command in: {stderr}");
    found
}

/// Runs each printed command in its real shell, with `zajil resume`
/// replaced by a command that only prints its argument, inside `work`.
/// Returns what each shell passed as the argument.
fn run_in_shells(commands: &[(String, String)], work: &Path) -> Vec<String> {
    let mut args = Vec::new();
    for (shell, command) in commands {
        let quoted = command
            .strip_prefix("zajil resume ")
            .unwrap_or_else(|| panic!("unexpected command: {command}"));
        let outputs: Vec<Output> = match shell.as_str() {
            "PowerShell" => {
                let script = work.join("t.ps1");
                fs::write(&script, format!("Write-Output {quoted}\r\n")).unwrap();
                vec![
                    Command::new("powershell")
                        .args([
                            "-NoProfile",
                            "-NonInteractive",
                            "-ExecutionPolicy",
                            "Bypass",
                            "-File",
                        ])
                        .arg(&script)
                        .current_dir(work)
                        .output()
                        .unwrap(),
                ]
            }
            "cmd" => {
                let script = work.join("t.cmd");
                fs::write(&script, format!("@echo off\r\necho {quoted}\r\n")).unwrap();
                vec![
                    Command::new("cmd")
                        .args(["/d", "/c"])
                        .arg(&script)
                        .current_dir(work)
                        .output()
                        .unwrap(),
                ]
            }
            _ => ["sh", "bash", "zsh"]
                .iter()
                .filter(|sh| Command::new(sh).args(["-c", "true"]).status().is_ok())
                .map(|sh| {
                    Command::new(sh)
                        .arg("-c")
                        .arg(format!("printf '%s' {quoted}"))
                        .current_dir(work)
                        .output()
                        .unwrap()
                })
                .collect(),
        };
        assert!(!outputs.is_empty(), "no shell found for {shell}");
        for out in outputs {
            assert!(out.status.success(), "{shell}: {}", text(&out.stderr));
            let printed = text(&out.stdout).trim_end().to_owned();
            // cmd's echo keeps the quotes.
            let printed = printed
                .strip_prefix('"')
                .and_then(|p| p.strip_suffix('"'))
                .map(str::to_owned)
                .unwrap_or(printed);
            args.push(printed);
        }
    }
    args
}

#[test]
fn an_interrupted_get_exits_3_and_resume_finishes_it() {
    let server = TestServer::http();
    let data = random_bytes(5_000_000, 2);
    server.put(
        "/f.bin",
        FileSpec::new(data.clone()).cut_after(1_500_000, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let dir_arg = dir.path().to_str().unwrap();

    // One connection, so the cut of the first response stops the download.
    let out = zajil(&["get", &server.url("/f.bin"), "-c", "1", "-d", dir_arg]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out.stderr));
    // The printed command is the one the user copies: run it in the shell
    // and resume with exactly the path the shell sees.
    let printed = run_in_shells(&printed_commands(&text(&out.stderr)), work.path());
    let state_file = only_state_file(dir.path());
    for path in &printed {
        assert_eq!(
            Path::new(path).canonicalize().unwrap(),
            state_file.canonicalize().unwrap()
        );
        assert!(!path.starts_with(r"\\?\"), "{path}");
    }

    let out = zajil(&["resume", &printed[0]]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("Resuming"));
    let saved = dir.path().join("f.bin");
    assert_eq!(sha256_hex(&fs::read(saved).unwrap()), sha256_hex(&data));
    assert!(!state_file.exists());
}

#[test]
fn the_printed_resume_command_cannot_run_code_from_the_file_name() {
    // Review of #18, reproduced: the server names the file
    // `$(touch PWNED)`id`.zip`. Inside the old double quotes, bash and zsh
    // ran `touch PWNED` and `id` when the user pasted the command.
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(1_000_000, 4))
            .content_disposition("attachment; filename=\"$(touch PWNED)`id`.zip\"")
            .cut_after(300_000, 1),
    );
    let dir = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();

    let out = zajil(&[
        "get",
        &server.url("/f.bin"),
        "--dir",
        dir.path().to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out.stderr));
    let state_file = only_state_file(dir.path());
    assert!(
        state_file
            .to_string_lossy()
            .contains("$(touch PWNED)`id`.zip"),
        "{}",
        state_file.display()
    );

    let commands = printed_commands(&text(&out.stderr));
    for path in run_in_shells(&commands, work.path()) {
        // The shell passed the name through untouched…
        assert_eq!(
            Path::new(&path).canonicalize().unwrap(),
            state_file.canonicalize().unwrap()
        );
        assert!(!path.contains("uid="), "`id` ran: {path}");
    }
    // …and ran nothing.
    assert!(!work.path().join("PWNED").exists(), "`touch PWNED` ran");
    assert!(!dir.path().join("PWNED").exists(), "`touch PWNED` ran");
}

#[test]
fn resume_works_with_a_relative_path() {
    // Review of #17/#18: a relative path finished the download, then exited
    // with 1 ("No such file or directory").
    let server = TestServer::http();
    let data = random_bytes(2_000_000, 5);
    server.put("/f.bin", FileSpec::new(data.clone()).cut_after(500_000, 1));
    let dir = tempfile::tempdir().unwrap();
    let out = zajil(&[
        "get",
        &server.url("/f.bin"),
        "--dir",
        dir.path().to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out.stderr));

    let out = Command::new(env!("CARGO_BIN_EXE_zajil"))
        .args(["resume", "f.bin.zajil-state"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(fs::read(dir.path().join("f.bin")).unwrap(), data);
    assert!(!dir.path().join("f.bin.zajil-state").exists());
}

#[test]
fn a_killed_download_resumes_from_the_state_file() {
    let server = TestServer::http();
    let data = random_bytes(20 * 1024 * 1024, 3);
    // Slow enough that we can kill it in the middle, with a fixed ETag so the
    // unthrottled replacement below is the same version of the file.
    server.put(
        "/big.bin",
        FileSpec::new(data.clone())
            .etag(Some("\"fixed\""))
            .throttle(2 * 1024 * 1024),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = dir.path().join("big.bin.zajil-state");

    let mut child = Command::new(env!("CARGO_BIN_EXE_zajil"))
        .args([
            "get",
            &server.url("/big.bin"),
            "--connections",
            "1",
            "--dir",
        ])
        .arg(dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // Wait until some progress is durable, then kill without warning.
    let deadline = Instant::now() + Duration::from_secs(60);
    let done_before_kill = loop {
        assert!(Instant::now() < deadline, "no progress was saved");
        if let Ok(state) = DownloadState::load(&state_file)
            && state.bytes_done() >= 2 * 1024 * 1024
        {
            break state.bytes_done();
        }
        assert!(child.try_wait().unwrap().is_none(), "zajil exited early");
        std::thread::sleep(Duration::from_millis(50));
    };
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!dir.path().join("big.bin").exists());

    server.put(
        "/big.bin",
        FileSpec::new(data.clone()).etag(Some("\"fixed\"")),
    );
    let out = zajil(&["resume", state_file.to_str().unwrap(), "--connections", "1"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let saved = dir.path().join("big.bin");
    assert_eq!(sha256_hex(&fs::read(&saved).unwrap()), sha256_hex(&data));

    // The resumed request started from what the state file had recorded.
    let resumed = server.gets("/big.bin").pop().unwrap();
    let from: u64 = resumed
        .range
        .as_deref()
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.strip_suffix('-'))
        .and_then(|n| n.parse().ok())
        .expect("a range request");
    assert!(from >= done_before_kill);
    assert_eq!(resumed.status, 206);
}

/// M2 success criterion: kill in the middle of a download of several
/// segments, then resume from the state file. The file is whole, and no byte
/// the state file counted was asked for again.
#[test]
fn a_killed_parallel_download_resumes_from_the_state_file() {
    let server = TestServer::http();
    let data = random_bytes(20 * 1024 * 1024, 4);
    // Each connection is slow, so the kill lands in the middle.
    server.put(
        "/big.bin",
        FileSpec::new(data.clone())
            .etag(Some("\"fixed\""))
            .throttle(512 * 1024),
    );
    let dir = tempfile::tempdir().unwrap();
    let state_file = dir.path().join("big.bin.zajil-state");

    let mut child = Command::new(env!("CARGO_BIN_EXE_zajil"))
        .args([
            "get",
            &server.url("/big.bin"),
            "--connections",
            "8",
            "--dir",
        ])
        .arg(dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // Wait until several segments have durable progress, then kill.
    let deadline = Instant::now() + Duration::from_secs(60);
    let before_kill = loop {
        assert!(Instant::now() < deadline, "no progress was saved");
        if let Ok(state) = DownloadState::load(&state_file)
            && state.segments.len() == 8
            && state.segments.iter().filter(|s| s.done > 0).count() >= 4
        {
            break state;
        }
        assert!(child.try_wait().unwrap().is_none(), "zajil exited early");
        std::thread::sleep(Duration::from_millis(50));
    };
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!dir.path().join("big.bin").exists());
    let requests_before = server.gets("/big.bin").len();

    server.put(
        "/big.bin",
        FileSpec::new(data.clone()).etag(Some("\"fixed\"")),
    );
    let out = zajil(&["resume", state_file.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let saved = dir.path().join("big.bin");
    assert_eq!(sha256_hex(&fs::read(&saved).unwrap()), sha256_hex(&data));
    assert!(!state_file.exists());

    // No resumed request covers bytes the state file had counted before the
    // kill (the state saved at the kill may count even more).
    for request in &server.gets("/big.bin")[requests_before..] {
        let (from, to) = request
            .range
            .as_deref()
            .and_then(|r| r.strip_prefix("bytes="))
            .and_then(|r| r.split_once('-'))
            .and_then(|(a, b)| Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?)))
            .expect("resume asks for ranges");
        assert_eq!(request.status, 206);
        for segment in &before_kill.segments {
            let counted = (segment.start, segment.start + segment.done);
            assert!(
                to < counted.0 || from >= counted.1,
                "bytes {from}-{to} were already on disk ({counted:?})"
            );
        }
    }
}

#[test]
fn connections_must_be_between_1_and_32() {
    for bad in ["0", "33", "many"] {
        let out = zajil(&["get", "http://127.0.0.1:9/x", "--connections", bad]);
        assert_eq!(out.status.code(), Some(2), "{bad}");
    }
}

#[test]
fn a_damaged_state_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let state_file = dir.path().join("x.bin.zajil-state");
    fs::write(
        &state_file,
        b"{\"format\":\"zajil-state\",\"schema_version\":1}",
    )
    .unwrap();
    let out = zajil(&["resume", state_file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("damaged"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn an_invalid_certificate_fails_clearly() {
    let server = TestServer::https_self_signed(&["localhost"], "localhost");
    server.put("/f.bin", FileSpec::new(vec![1u8; 10]));
    let dir = tempfile::tempdir().unwrap();
    let out = zajil(&[
        "get",
        &server.url("/f.bin"),
        "--dir",
        dir.path().to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = text(&out.stderr).to_lowercase();
    assert!(stderr.contains("certificate"), "{stderr}");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn help_offers_no_way_to_skip_certificate_checks() {
    for args in [&["--help"][..], &["get", "--help"], &["resume", "--help"]] {
        let out = zajil(args);
        assert!(out.status.success());
        let help = text(&out.stdout).to_lowercase();
        for word in ["insecure", "no-verify", "danger", "accept-invalid", "skip"] {
            assert!(!help.contains(word), "{args:?} mentions {word}");
        }
    }
    let help = text(&zajil(&["--help"]).stdout);
    assert!(help.contains("get") && help.contains("resume"));
}

#[test]
fn wrong_usage_exits_2() {
    assert_eq!(zajil(&[]).status.code(), Some(2));
    assert_eq!(zajil(&["get"]).status.code(), Some(2));
    assert_eq!(zajil(&["get", "x", "--insecure"]).status.code(), Some(2));
}
