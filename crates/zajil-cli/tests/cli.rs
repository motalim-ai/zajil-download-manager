//! End-to-end tests of the `zajil` binary against the local test server.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use zajil_core::state::DownloadState;
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

fn zajil(args: &[&str]) -> Output {
    common::zajil_command()
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

/// The POSIX shells that printed commands are run in. On CI all three must
/// be there (zsh is installed on Linux for it): a missing shell fails the
/// test instead of skipping it silently (owner's decision on #28). Locally
/// a missing one is skipped.
fn posix_shells() -> Vec<&'static str> {
    let found: Vec<&str> = ["sh", "bash", "zsh"]
        .into_iter()
        .filter(|sh| Command::new(sh).args(["-c", "true"]).status().is_ok())
        .collect();
    if std::env::var_os("CI").is_some() {
        assert_eq!(found, ["sh", "bash", "zsh"], "a shell is missing on CI");
    }
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
            _ => posix_shells()
                .into_iter()
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

    // One connection and one try, so the cut of the first response stops
    // the download.
    let out = zajil(&[
        "get",
        &server.url("/f.bin"),
        "-c",
        "1",
        "--tries",
        "1",
        "-d",
        dir_arg,
    ]);
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
        "--tries",
        "1",
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
        "--tries",
        "1",
        "--dir",
        dir.path().to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out.stderr));

    let out = common::zajil_command()
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

    let mut child = common::zajil_command()
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

    let mut child = common::zajil_command()
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

/// Owner decision on #28: a resume that needs a user name and password
/// says so and how to download the file again. Review of #29:
/// - following the steps gives the file its own name, with nothing left
///   behind;
/// - nothing secret is printed: not the password, not a token in the URL;
/// - the paths come from the server (here `$(touch PWNED)`, backticks and a
///   space, next to a file of the user's named `x`), so the printed delete
///   command and folder are run in each real shell: they delete exactly the
///   two files, run nothing, and pass the folder through untouched.
#[test]
fn a_resume_that_needs_credentials_says_how_to_download_again() {
    let server = TestServer::http();
    let data = random_bytes(1_000_000, 6);
    let name = "x $(touch PWNED)`id`.bin";
    let spec = || {
        FileSpec::new(data.clone())
            .etag(Some("\"same\""))
            .require_authorization("Basic dTpw")
            .content_disposition(&format!("attachment; filename=\"{name}\""))
    };
    server.put("/f.bin", spec().cut_after(300_000, 1));
    // The folder's name is tricky too, so `--dir` must be quoted (third
    // review of #29): a space, `$(touch PWNED)`, a backtick and a quote.
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("d $(touch PWNED)`id` it's");
    fs::create_dir(&dir).unwrap();
    let dir_arg = dir.to_str().unwrap();
    fs::write(dir.as_path().join("x"), b"the user's own file").unwrap();
    let url = format!(
        "{}?sig=SECRET-TOKEN",
        server.url("/f.bin").replacen("http://", "http://u:p@", 1)
    );

    let out = zajil(&["get", &url, "-c", "1", "--tries", "1", "-d", dir_arg]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out.stderr));
    let state_file = only_state_file(dir.as_path());
    let part_file = dir.as_path().join(format!("{name}.zajil-part"));
    assert!(part_file.exists());

    let out = zajil(&["resume", state_file.to_str().unwrap()]);
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("asks for a user name and password"),
        "{stderr}"
    );
    for secret in ["u:p@", ":p@", "SECRET-TOKEN"] {
        assert!(!stderr.contains(secret), "{secret} printed: {stderr}");
    }

    // The printed commands: step 1 (delete) and step 2 (the folder).
    let (mut deletes, mut folders) = (Vec::new(), Vec::new());
    let mut step = 0;
    let mut lines = stderr.lines();
    while let Some(line) = lines.next() {
        if line.starts_with("zajil:   1.") {
            step = 1;
        } else if line.starts_with("zajil:   2.") {
            step = 2;
        } else if let Some(shell) = line
            .strip_prefix("zajil:      in ")
            .and_then(|rest| rest.strip_suffix(':'))
        {
            let command = lines.next().unwrap().trim_start().to_owned();
            match step {
                1 => deletes.push((shell.to_owned(), command)),
                2 => folders.push((shell.to_owned(), command)),
                _ => panic!("a command outside the steps: {stderr}"),
            }
        }
    }
    assert!(!deletes.is_empty() && !folders.is_empty(), "{stderr}");

    let state_bytes = fs::read(&state_file).unwrap();
    let part_bytes = fs::read(&part_file).unwrap();
    for (shell, command) in &deletes {
        let check = |out: Output| {
            assert!(out.status.success(), "{shell}: {}", text(&out.stderr));
            assert!(
                !state_file.exists() && !part_file.exists(),
                "{shell}: {command}"
            );
            assert!(!dir.as_path().join("PWNED").exists(), "{shell} ran code");
            assert!(!root.path().join("PWNED").exists(), "{shell} ran code");
            assert_eq!(
                fs::read(dir.as_path().join("x")).unwrap(),
                b"the user's own file"
            );
            // Put them back for the next shell.
            fs::write(&state_file, &state_bytes).unwrap();
            fs::write(&part_file, &part_bytes).unwrap();
        };
        assert!(
            run_in_shell(shell, command, dir.as_path(), check) > 0,
            "no {shell}"
        );
    }
    for (shell, command) in &folders {
        let args = command
            .strip_prefix("--dir ")
            .unwrap_or_else(|| panic!("unexpected: {command}"));
        let check = |out: Output| {
            assert!(out.status.success(), "{shell}: {}", text(&out.stderr));
            let printed = text(&out.stdout).trim_end().to_owned();
            assert_eq!(
                Path::new(&printed).canonicalize().unwrap(),
                dir.as_path().canonicalize().unwrap(),
                "{shell}"
            );
        };
        let shown = run_in_shell(shell, &format!("show {args}"), dir.as_path(), check);
        assert!(shown > 0, "no {shell}");
    }

    // Follow the steps: delete both files, run the same `zajil get`.
    fs::remove_file(&state_file).unwrap();
    fs::remove_file(&part_file).unwrap();
    server.put("/f.bin", spec());
    let out = zajil(&["get", &url, "-c", "1", "-d", dir_arg]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let mut names: Vec<_> = fs::read_dir(dir.as_path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["x", name]);
    assert_eq!(fs::read(dir.as_path().join(name)).unwrap(), data);
}

/// Runs `command` in `shell` (as `zajil` names it) inside `work`, once per
/// real shell, and hands each result to `check` before the next run.
/// `show <arg>` prints its one argument, in every shell. Returns how many
/// shells ran.
fn run_in_shell(shell: &str, command: &str, work: &Path, check: impl Fn(Output)) -> usize {
    let script_dir = work.parent().unwrap().to_path_buf();
    let work = work.to_path_buf();
    let runs: Vec<Box<dyn Fn() -> Output>> = match shell {
        "PowerShell" => {
            let script = script_dir.join("zajil-test.ps1");
            fs::write(
                &script,
                format!("function show {{ Write-Output $args[0] }}\r\n{command}\r\n"),
            )
            .unwrap();
            vec![Box::new(move || {
                Command::new("powershell")
                    .args([
                        "-NoProfile",
                        "-NonInteractive",
                        "-ExecutionPolicy",
                        "Bypass",
                        "-File",
                    ])
                    .arg(&script)
                    .current_dir(&work)
                    .output()
                    .unwrap()
            })]
        }
        "cmd" => {
            let script = script_dir.join("zajil-test.cmd");
            let command = command.replacen("show ", "echo ", 1);
            fs::write(&script, format!("@echo off\r\n{command}\r\n")).unwrap();
            vec![Box::new(move || {
                let mut out = Command::new("cmd")
                    .args(["/d", "/c"])
                    .arg(&script)
                    .current_dir(&work)
                    .output()
                    .unwrap();
                // cmd's echo keeps the quotes.
                let shown = text(&out.stdout).trim_end().trim_matches('"').to_owned();
                out.stdout = shown.into_bytes();
                out
            })]
        }
        _ => posix_shells()
            .into_iter()
            .map(|sh| {
                let script = format!("show() {{ printf '%s' \"$1\"; }}; {command}");
                let work = work.clone();
                Box::new(move || {
                    Command::new(sh)
                        .arg("-c")
                        .arg(&script)
                        .current_dir(&work)
                        .output()
                        .unwrap()
                }) as Box<dyn Fn() -> Output>
            })
            .collect(),
    };
    for run in &runs {
        check(run());
    }
    runs.len()
}

/// Review of #29: an invalid URL printed in full, password included.
#[test]
fn an_invalid_url_never_prints_its_password() {
    let out = zajil(&["get", "http://u:S3cret@127.0.0.1:99999/f.bin", "-d", "."]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = text(&out.stderr);
    assert!(stderr.contains("invalid URL"), "{stderr}");
    assert!(!stderr.contains("S3cret"), "{stderr}");
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

/// Owner's decision on #28: a token in the URL (`?sig=…`) never appears in
/// what `zajil` prints: retry messages, a stopped download, a connection
/// error, an invalid URL.
#[test]
fn zajil_never_prints_a_token_from_the_url() {
    const TOKEN: &str = "SECRET-TOKEN";
    let server = TestServer::http();
    let spec = || FileSpec::new(random_bytes(500_000, 12)).cut_after(100_000, 1);
    let dir = tempfile::tempdir().unwrap();
    let dir_arg = dir.path().to_str().unwrap();
    let with_token = |base: &str| format!("{base}?sig={TOKEN}#{TOKEN}");
    let cases: [(String, &str, Option<i32>); 4] = [
        // Cut, tried again, finished.
        (with_token(&server.url("/f.bin")), "2", Some(0)),
        // Cut, stopped: resumable.
        (with_token(&server.url("/g.bin")), "1", Some(3)),
        // No server there: tried again, then failed.
        (with_token("http://127.0.0.1:1/f.bin"), "2", Some(1)),
        (with_token("http://127.0.0.1:99999/f.bin"), "2", Some(1)),
    ];
    server.put("/f.bin", spec());
    server.put("/g.bin", spec());
    for (url, tries, code) in cases {
        let out = zajil(&["get", &url, "-c", "1", "--tries", tries, "-d", dir_arg]);
        let printed = format!("{}{}", text(&out.stdout), text(&out.stderr));
        assert_eq!(out.status.code(), code, "{printed}");
        assert!(!printed.contains(TOKEN), "{printed}");
    }
}
