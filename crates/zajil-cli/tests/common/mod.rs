//! What every test of the `zajil` command shares.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

/// The `zajil` command, for tests. It always runs with
/// `ZAJIL_NO_POWER_ACTIONS` set, so no test can shut down or put to sleep
/// the machine it runs on, whatever it asks. `power_guard.rs` checks that
/// every test starts `zajil` through here.
pub fn zajil_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_zajil"));
    command.env(zajil_platform::power::NO_POWER_ACTIONS, "1");
    command
}

/// A `zajil` a test started in the background: killed and waited for when
/// the test ends, also when it fails before its own `kill` (#50: a process
/// left running can keep connecting to a port number that a later test's
/// server reuses). Use it as the `Child` it holds.
/// Starts `command` in the background, held by [`Running`]: the only way
/// tests start a process they do not wait for at once (`process_guard.rs`
/// checks that no test file calls `spawn` itself).
#[allow(dead_code)] // Not every test file starts `zajil` in the background.
pub fn spawn(command: &mut Command) -> Running {
    Running(command.spawn().expect("zajil starts"))
}

#[allow(dead_code)] // Not every test file starts `zajil` in the background.
pub struct Running(pub std::process::Child);

impl std::ops::Deref for Running {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Running {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // Already ended: kill fails harmlessly, and wait returns at once.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The resume commands `zajil` printed, by shell: lines like
/// `zajil: to resume, run this in <shell>:` followed by the indented
/// command.
#[allow(dead_code)] // Not every test file prints a resume command.
pub fn printed_commands(stderr: &str) -> Vec<(String, String)> {
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
#[allow(dead_code)] // Not every test file runs commands in shells.
pub fn posix_shells() -> Vec<&'static str> {
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
#[allow(dead_code)] // Not every test file prints a resume command.
pub fn run_in_shells(commands: &[(String, String)], work: &Path) -> Vec<String> {
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
