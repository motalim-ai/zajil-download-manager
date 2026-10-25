//! `zajil-jsbox`: what yt-dlp runs as QuickJS-NG, to answer YouTube's
//! JavaScript challenges (ADR-0009, owner's conditions).
//!
//! Zajil gives yt-dlp `--js-runtimes quickjs:<this program>`, and the path
//! of the real `qjs` in `ZAJIL_QJS` with its pinned SHA-256 in
//! `ZAJIL_QJS_SHA256`, checked again here right before it runs. yt-dlp then runs this program in two ways only, and
//! anything else is refused:
//!
//! - `--help`: yt-dlp reads QuickJS-NG's version from it.
//! - `--script <file>`: the challenge, which the page's JavaScript is part
//!   of. The file is copied into a new, empty, private folder, and `qjs`
//!   runs it there:
//!   - in the sandbox (`zajil_platform::sandbox`): no other program,
//!     writing only in that folder, and no network, where the system
//!     allows it;
//!   - with no environment (`SystemRoot` alone on Windows, to start);
//!   - with QuickJS's own memory and stack limits, and a time limit after
//!     which it is ended. It stays in yt-dlp's process tree, with Zajil's
//!     mark, so stopping the run ends it too.
//!
//! What the sandbox holds on this system is written to a new file in the
//! folder named by `ZAJIL_JSBOX_REPORT`, for Zajil to tell the user when it
//! is not everything. Nothing is printed but what `qjs` prints: yt-dlp
//! takes any output on stderr as an error.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use zajil_platform::sandbox::{Confinement, Sandbox};

/// The largest challenge read (yt-dlp's solver and YouTube's player).
const MAX_SCRIPT: u64 = 32 * 1024 * 1024;
/// The most `qjs` may print on each of stdout and stderr.
const MAX_OUTPUT: u64 = 16 * 1024 * 1024;
/// QuickJS's own limits, in KiB.
const MEMORY_KIB: &str = "1048576";
const STACK_KIB: &str = "8192";
/// How long a challenge may run, and `--help`.
const SCRIPT_TIME: Duration = Duration::from_secs(60);
const HELP_TIME: Duration = Duration::from_secs(10);

fn main() -> ExitCode {
    match run(std::env::args_os().skip(1).collect()) {
        Ok(code) => code,
        Err(err) => {
            // Only on failure: yt-dlp reports it as QuickJS's error.
            let _ = writeln!(io::stderr(), "zajil-jsbox: {err}");
            ExitCode::from(2)
        }
    }
}

/// What yt-dlp asked for.
enum Asked {
    Help,
    Script(PathBuf),
}

fn asked(args: &[OsString]) -> io::Result<Asked> {
    match args {
        [help] if help == "--help" => Ok(Asked::Help),
        [flag, file] if flag == "--script" => Ok(Asked::Script(PathBuf::from(file))),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "only --help and --script <file> are accepted",
        )),
    }
}

fn random_name() -> io::Result<String> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn run(args: Vec<OsString>) -> io::Result<ExitCode> {
    let asked = asked(&args)?;
    let qjs = PathBuf::from(
        std::env::var_os("ZAJIL_QJS")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "ZAJIL_QJS is not set"))?,
    );
    if !qjs.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ZAJIL_QJS is not an absolute path",
        ));
    }
    // Checked again here, right before it runs (Zajil checked it when the
    // run began).
    let expected = std::env::var("ZAJIL_QJS_SHA256")
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "ZAJIL_QJS_SHA256 is not set"))?;
    check_sha256(&qjs, &expected)?;
    // A new, empty, private folder in the run's temporary folder (TMPDIR,
    // set by Zajil), for this challenge only.
    let work = std::env::temp_dir().join(format!("js-{}", random_name()?));
    zajil_platform::fs::private_dir(&work)?;
    let mut qjs_args: Vec<OsString> = vec![
        "--memory-limit".into(),
        MEMORY_KIB.into(),
        "--stack-size".into(),
        STACK_KIB.into(),
    ];
    let time = match &asked {
        Asked::Help => {
            qjs_args.push("--help".into());
            HELP_TIME
        }
        Asked::Script(file) => {
            copy_script(file, &work.join("challenge.js"))?;
            qjs_args.extend(["--script".into(), "challenge.js".into()]);
            script_time()
        }
    };
    // Opened before the sandbox, which may refuse it after.
    let report = match std::env::var_os("ZAJIL_JSBOX_REPORT") {
        Some(dir) => {
            let path = Path::new(&dir).join(format!("{}.txt", random_name()?));
            let file = zajil_platform::fs::create_private(&path)?;
            Some((path, file))
        }
        None => None,
    };
    // No thread before this (Landlock confines the calling thread).
    let sandbox = match Sandbox::enter(&work, &qjs) {
        Ok(sandbox) => sandbox,
        Err(err) => {
            // qjs never ran: no report, rather than an empty one, which
            // would read as "ran with no sandbox".
            if let Some((path, file)) = report {
                drop(file);
                let _ = std::fs::remove_file(path);
            }
            return Err(err);
        }
    };
    let report = report.map(|(_, file)| file);
    if let Some(mut report) = report {
        report.write_all(report_line(sandbox.confinement()).as_bytes())?;
    }
    let mut command = sandbox.command();
    command
        .args(&qjs_args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Only what a program needs to start on Windows, and Zajil's mark of
    // this run's tools, so that ending the run (or the next start, after
    // a crash) ends `qjs` too.
    for name in ["SystemRoot", zajil_platform::process::TOKEN_VAR] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let (status, stdout, stderr) = run_for(&sandbox, command, time)?;
    // The challenge goes now, not with the run (where the sandbox lets it:
    // the folder itself may stay, empty, until then).
    let _ = std::fs::remove_dir_all(&work);
    io::stdout().write_all(&stdout)?;
    io::stderr().write_all(&stderr)?;
    Ok(match status {
        Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        None => {
            let _ = writeln!(
                io::stderr(),
                "zajil-jsbox: QuickJS ran longer than {} seconds",
                time.as_secs()
            );
            ExitCode::from(1)
        }
    })
}

/// How long a challenge may run: [`SCRIPT_TIME`], or less if
/// `ZAJIL_JSBOX_TIME_MS` says so (the tests); never more.
fn script_time() -> Duration {
    time_limit(std::env::var("ZAJIL_JSBOX_TIME_MS").ok().as_deref())
}

fn time_limit(asked: Option<&str>) -> Duration {
    asked
        .and_then(|ms| ms.parse().ok())
        .map(Duration::from_millis)
        .map_or(SCRIPT_TIME, |shorter| shorter.min(SCRIPT_TIME))
}

/// The largest `qjs` read to check (QuickJS-NG's are about 2.5 MiB).
const MAX_QJS: u64 = 64 * 1024 * 1024;

/// Refuses `qjs` unless its SHA-256 is `expected` (lower-case hex).
fn check_sha256(qjs: &Path, expected: &str) -> io::Result<()> {
    let mut file = zajil_platform::fs::open_regular_nofollow(qjs, false)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n as u64);
        if total > MAX_QJS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "qjs is too large",
            ));
        }
        hasher.update(&buffer[..n]);
    }
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("qjs is not the pinned file (SHA-256 {actual})"),
        ));
    }
    Ok(())
}

/// The report's line: what holds.
fn report_line(confinement: Confinement) -> String {
    format!(
        "programs={} files={} network={}\n",
        confinement.programs, confinement.files, confinement.network
    )
}

/// Copies yt-dlp's challenge file, refusing one larger than [`MAX_SCRIPT`].
fn copy_script(from: &Path, to: &Path) -> io::Result<()> {
    let mut text = Vec::new();
    // Never through a link (AGENTS.md, section 10).
    zajil_platform::fs::open_regular_nofollow(from, false)?
        .take(MAX_SCRIPT + 1)
        .read_to_end(&mut text)?;
    if text.len() as u64 > MAX_SCRIPT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the challenge is larger than Zajil accepts",
        ));
    }
    zajil_platform::fs::create_private(to)?.write_all(&text)
}

/// Reads at most [`MAX_OUTPUT`] bytes of `from`, and drains the rest.
fn read_capped(from: impl Read) -> Vec<u8> {
    let mut from = from;
    let mut out = Vec::new();
    let _ = (&mut from).take(MAX_OUTPUT).read_to_end(&mut out);
    let _ = io::copy(&mut from, &mut io::sink());
    out
}

/// How long the output may still come once `qjs` has ended (it can start
/// nothing that would keep its pipes open, but no wait is unbounded).
const DRAIN_TIME: Duration = Duration::from_secs(2);

/// Reads all of `from` (capped) on a thread of its own; the result comes
/// on the returned channel.
fn reader(from: impl Read + Send + 'static) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(read_capped(from));
    });
    receive
}

/// Runs `command` in the sandbox for at most `time`: its exit code (`None`
/// when it ran too long and was ended) and output.
fn run_for(
    sandbox: &Sandbox,
    command: std::process::Command,
    time: Duration,
) -> io::Result<(Option<i32>, Vec<u8>, Vec<u8>)> {
    let mut confined = sandbox.spawn(command)?;
    let child = confined.child();
    let out = reader(child.stdout.take().expect("piped"));
    let err = reader(child.stderr.take().expect("piped"));
    let deadline = Instant::now() + time;
    let status = loop {
        let ended = match child.try_wait() {
            Ok(ended) => ended,
            Err(err) => {
                // Never left running without its time limit.
                let _ = child.kill();
                let _ = child.wait();
                return Err(err);
            }
        };
        if let Some(status) = ended {
            break Some(status.code().unwrap_or(1));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let out = out.recv_timeout(DRAIN_TIME).unwrap_or_default();
    let err = err.recv_timeout(DRAIN_TIME).unwrap_or_default();
    Ok((status, out, err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_limit_can_only_be_shortened() {
        assert_eq!(time_limit(None), SCRIPT_TIME);
        assert_eq!(time_limit(Some("1500")), Duration::from_millis(1500));
        assert_eq!(time_limit(Some("999999999999")), SCRIPT_TIME);
        assert_eq!(time_limit(Some("not a number")), SCRIPT_TIME);
        assert_eq!(time_limit(Some("-5")), SCRIPT_TIME);
    }

    #[test]
    fn only_the_two_ways_yt_dlp_runs_quickjs_are_accepted() {
        let os = |args: &[&str]| args.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(matches!(asked(&os(&["--help"])), Ok(Asked::Help)));
        assert!(matches!(
            asked(&os(&["--script", "/tmp/a.js"])),
            Ok(Asked::Script(_))
        ));
        for refused in [
            &[][..],
            &["-e", "1"][..],
            &["--std", "--script", "a.js"][..],
            &["--script"][..],
            &["--script", "a.js", "b.js"][..],
            &["a.js"][..],
            &["--help", "--script", "a.js"][..],
        ] {
            assert!(asked(&os(refused)).is_err(), "{refused:?}");
        }
    }
}
