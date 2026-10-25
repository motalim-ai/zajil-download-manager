//! `zajil-jsbox` with the real QuickJS-NG (`ZAJIL_TEST_QJS`, the pinned
//! `qjs`) and the real yt-dlp (`ZAJIL_TEST_YT_DLP`). Without them the tests
//! are not run (and say so); on the CI they must be there.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The real tools, or `None` (not run) away from the CI.
fn tool(var: &str) -> Option<PathBuf> {
    match std::env::var_os(var) {
        Some(path) => Some(PathBuf::from(path)),
        None if std::env::var_os("CI").is_some() => panic!("{var} must be set on the CI"),
        None => {
            eprintln!("NOT RUN: set {var} to run the real zajil-jsbox tests");
            None
        }
    }
}

/// A private run folder, as Zajil makes one: TMPDIR and the reports.
struct Run {
    dir: tempfile::TempDir,
}

impl Run {
    fn new() -> Run {
        let dir = tempfile::tempdir().unwrap();
        zajil_platform::fs::private_dir(&dir.path().join("tmp")).unwrap();
        zajil_platform::fs::private_dir(&dir.path().join("reports")).unwrap();
        Run { dir }
    }

    fn jsbox(&self, qjs: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_zajil-jsbox"));
        command
            .env("ZAJIL_QJS", qjs)
            .env("ZAJIL_QJS_SHA256", sha256(qjs))
            .env("ZAJIL_JSBOX_REPORT", self.dir.path().join("reports"))
            .env("TMPDIR", self.dir.path().join("tmp"))
            .env("TMP", self.dir.path().join("tmp"))
            .env("TEMP", self.dir.path().join("tmp"));
        command
    }

    fn script(&self, name: &str, text: &str) -> PathBuf {
        let file = self.dir.path().join(name);
        std::fs::write(&file, text).unwrap();
        file
    }

    fn report(&self) -> String {
        let mut all = String::new();
        for entry in std::fs::read_dir(self.dir.path().join("reports")).unwrap() {
            all.push_str(&std::fs::read_to_string(entry.unwrap().path()).unwrap());
        }
        all
    }
}

/// The SHA-256 of `file`, as Zajil pins it.
fn sha256(file: &Path) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(std::fs::read(file).unwrap());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn text(output: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_challenge_runs_and_its_answer_comes_back() {
    let Some(qjs) = tool("ZAJIL_TEST_QJS") else {
        return;
    };
    let run = Run::new();
    let script = run.script("a.js", "console.log(JSON.stringify({ answer: 6 * 7 }));\n");
    let output = run
        .jsbox(&qjs)
        .args(["--script".as_ref(), script.as_os_str()])
        .output()
        .unwrap();
    let (out, err) = text(&output);
    assert!(output.status.success(), "{out}{err}");
    assert_eq!(out.trim(), r#"{"answer":42}"#);
    // Nothing on stderr: yt-dlp takes any as an error.
    assert_eq!(err, "");
    assert!(run.report().contains("files="), "{}", run.report());
}

#[test]
fn help_shows_quickjs_ng_s_version_for_yt_dlp() {
    let Some(qjs) = tool("ZAJIL_TEST_QJS") else {
        return;
    };
    let run = Run::new();
    let output = run.jsbox(&qjs).arg("--help").output().unwrap();
    assert!(
        text(&output).0.starts_with("QuickJS-ng version 0."),
        "{:?}",
        text(&output)
    );
}

#[test]
fn anything_else_is_refused_before_qjs_runs() {
    let Some(qjs) = tool("ZAJIL_TEST_QJS") else {
        return;
    };
    let run = Run::new();
    for args in [
        &["-e", "1"][..],
        &["--std", "--script", "a.js"][..],
        &[][..],
    ] {
        let output = run.jsbox(&qjs).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(text(&output).0, "");
    }
    assert_eq!(run.report(), "", "the sandbox was never entered");
}

#[test]
fn a_challenge_cannot_write_outside_its_folder_even_through_qjs_os() {
    let Some(qjs) = tool("ZAJIL_TEST_QJS") else {
        return;
    };
    let run = Run::new();
    let elsewhere = tempfile::tempdir().unwrap();
    let outside = elsewhere.path().join("PWNED");
    let path = outside.to_string_lossy().replace('\\', "\\\\");
    // QuickJS-NG's CLI always lets a script import its `std` and `os`
    // modules: the sandbox, not QuickJS, is what holds.
    let script = run.script(
        "evil.js",
        &format!(
            "import('qjs:std').then(std => {{\n\
             const f = std.open(\"{path}\", 'w');\n\
             if (f) {{ f.puts('x'); f.close(); }}\n\
             console.log(f ? 'wrote' : 'refused');\n\
             }});\n"
        ),
    );
    let output = run
        .jsbox(&qjs)
        .args(["--script".as_ref(), script.as_os_str()])
        .output()
        .unwrap();
    let (out, err) = text(&output);
    let report = run.report();
    if report.contains("files=true") {
        assert_eq!(out.trim(), "refused", "{out}{err}");
        assert!(!outside.exists());
    } else {
        assert!(
            std::env::var_os("CI").is_none(),
            "no file sandbox on CI: {report}"
        );
        eprintln!("NOT RUN: this system has no file sandbox ({report})");
    }
}

#[test]
fn a_challenge_that_runs_too_long_is_ended() {
    let Some(qjs) = tool("ZAJIL_TEST_QJS") else {
        return;
    };
    let run = Run::new();
    let script = run.script("loop.js", "for (;;) {}\n");
    let started = std::time::Instant::now();
    let output = run
        .jsbox(&qjs)
        .env("ZAJIL_JSBOX_TIME_MS", "1500")
        .args(["--script".as_ref(), script.as_os_str()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output).1.contains("ran longer than"),
        "{:?}",
        text(&output)
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
}

#[test]
fn a_challenge_that_takes_too_much_memory_is_stopped_by_quickjs() {
    let Some(qjs) = tool("ZAJIL_TEST_QJS") else {
        return;
    };
    let run = Run::new();
    // More than the 1 GiB limit, in strings that cannot be shared.
    let script = run.script(
        "big.js",
        "const a = []; for (let i = 0; i < 4096; i++) a.push('x'.repeat(1 << 20) + i);\n\
         console.log('done ' + a.length);\n",
    );
    let output = run
        .jsbox(&qjs)
        .args(["--script".as_ref(), script.as_os_str()])
        .output()
        .unwrap();
    let (out, err) = text(&output);
    assert!(!out.contains("done"), "{out}{err}");
    assert!(!output.status.success(), "{out}{err}");
}

#[test]
fn yt_dlp_finds_quickjs_ng_through_the_box() {
    let (Some(qjs), Some(yt_dlp)) = (tool("ZAJIL_TEST_QJS"), tool("ZAJIL_TEST_YT_DLP")) else {
        return;
    };
    let run = Run::new();
    let mut runtime = std::ffi::OsString::from("quickjs:");
    runtime.push(env!("CARGO_BIN_EXE_zajil-jsbox"));
    let output = Command::new(yt_dlp)
        .args([
            "--ignore-config",
            "--no-plugin-dirs",
            "--no-remote-components",
            "--no-js-runtimes",
        ])
        .arg("--js-runtimes")
        .arg(runtime)
        .arg("--verbose")
        .current_dir(run.dir.path())
        .env("ZAJIL_QJS", &qjs)
        .env("ZAJIL_QJS_SHA256", sha256(&qjs))
        .env("ZAJIL_JSBOX_REPORT", run.dir.path().join("reports"))
        .env("TMPDIR", run.dir.path().join("tmp"))
        .output()
        .unwrap();
    let err = text(&output).1;
    assert!(
        err.lines()
            .any(|l| l.starts_with("[debug] JS runtimes: quickjs-ng-0.")),
        "{err}"
    );
}

#[test]
fn a_qjs_that_is_not_the_pinned_one_is_refused() {
    let Some(qjs) = tool("ZAJIL_TEST_QJS") else {
        return;
    };
    let run = Run::new();
    let output = run
        .jsbox(&qjs)
        .env("ZAJIL_QJS_SHA256", "0".repeat(64))
        .arg("--help")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        text(&output).1.contains("not the pinned file"),
        "{:?}",
        text(&output)
    );
    assert_eq!(run.report(), "", "nothing ran");
}

#[test]
fn a_challenge_cannot_start_a_program() {
    let Some(qjs) = tool("ZAJIL_TEST_QJS") else {
        return;
    };
    let run = Run::new();
    let elsewhere = tempfile::tempdir().unwrap();
    let outside = elsewhere.path().join("PWNED");
    // A shell that would write outside, through `qjs:os`, which QuickJS-NG's
    // CLI always offers. (Only `SystemRoot` is left in the environment.)
    let template = r#"
Promise.all([import('qjs:os'), import('qjs:std')]).then(([os, std]) => {
  const target = OUTSIDE;
  const shell = os.platform === 'win32'
    ? [std.getenv('SystemRoot') + '\\System32\\cmd.exe', '/c', 'type nul > "' + target + '"']
    : ['/bin/sh', '-c', 'echo x > "' + target + '"'];
  // Windows builds of QuickJS-NG have no os.exec at all: there the
  // one-process job is proven by zajil-platform's tests/sandbox.rs.
  if (typeof os.exec !== 'function') { console.log('absent'); return; }
  let code;
  try { code = os.exec(shell); } catch (e) { code = 'threw'; }
  console.log(code === 0 ? 'ran' : 'refused ' + code);
});
"#;
    let quoted = format!("{:?}", outside.to_string_lossy());
    let script = run.script("exec.js", &template.replace("OUTSIDE", &quoted));
    let output = run
        .jsbox(&qjs)
        .args(["--script".as_ref(), script.as_os_str()])
        .output()
        .unwrap();
    let (out, err) = text(&output);
    let report = run.report();
    if cfg!(windows) {
        assert_eq!(out.trim(), "absent", "{err}");
        assert!(!outside.exists());
    } else if report.contains("programs=true") {
        assert!(out.trim().starts_with("refused"), "{out}{err}");
        assert!(!outside.exists());
    } else {
        assert!(
            std::env::var_os("CI").is_none(),
            "no program ban on CI: {report}"
        );
        eprintln!("NOT RUN: this system cannot refuse other programs ({report})");
    }
}

/// The processes whose command line holds `marker` (`pgrep -f`).
#[cfg(unix)]
fn running_with(marker: &str) -> Vec<String> {
    let out = Command::new("pgrep").args(["-f", marker]).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

// Unix only: Windows builds of QuickJS-NG have no os.exec (see above).
#[cfg(unix)]
#[test]
fn a_challenge_cannot_run_qjs_again_without_its_limits() {
    let Some(qjs) = tool("ZAJIL_TEST_QJS") else {
        return;
    };
    let run = Run::new();
    // Landlock lets the box run qjs, so a challenge could start it again,
    // without the limits, the time limit or the run's token, and leave it
    // running after the box (review of the YouTube PR).
    let marker = format!("zajil-escape-{}", std::process::id());
    let template = r#"
import('qjs:os').then((os) => {
  let code;
  try {
    code = os.exec([QJS, '-e', 'for (;;) {} // MARKER'], { block: false, env: {} });
  } catch (e) { code = 'threw'; }
  console.log(typeof code === 'number' && code > 0 ? 'started ' + code : 'refused ' + code);
});
"#;
    let script = run.script(
        "again.js",
        &template
            .replace("QJS", &format!("{:?}", qjs.to_string_lossy()))
            .replace("MARKER", &marker),
    );
    let output = run
        .jsbox(&qjs)
        .args(["--script".as_ref(), script.as_os_str()])
        .output()
        .unwrap();
    let (out, err) = text(&output);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let left = running_with(&marker);
    for pid in &left {
        let _ = Command::new("kill").args(["-9", pid]).status();
    }
    let report = run.report();
    if report.contains("programs=true") {
        assert!(out.trim().starts_with("refused"), "{out}{err}");
        assert!(left.is_empty(), "a qjs outlived the box: {left:?}");
    } else {
        assert!(
            std::env::var_os("CI").is_none(),
            "no program ban on CI: {report}"
        );
        eprintln!("NOT RUN: this system cannot refuse other programs ({report})");
    }
}

#[test]
fn stopping_the_run_s_tree_ends_qjs_too() {
    let Some(qjs) = tool("ZAJIL_TEST_QJS") else {
        return;
    };
    let run = Run::new();
    // A challenge that never ends, ignores the signals it can, and beats
    // in its folder.
    let script = run.script(
        "beat.js",
        "Promise.all([import('qjs:os'), import('qjs:std')]).then(([os, std]) => {\n\
         try { os.signal(os.SIGTERM, () => {}); os.signal(os.SIGINT, () => {}); } catch (e) {}\n\
         for (;;) { const f = std.open('beat', 'w'); f.puts(String(Date.now())); f.close(); os.sleep(20); }\n\
         });\n",
    );
    let mut command = run.jsbox(&qjs);
    command.args(["--script".as_ref(), script.as_os_str()]);
    // As Zajil runs yt-dlp, which runs the box: one tree.
    let mut tree = zajil_platform::process::spawn(command).unwrap();
    let beat = || {
        std::fs::read_dir(run.dir.path().join("tmp"))
            .unwrap()
            .flatten()
            .map(|e| e.path().join("beat"))
            .find(|p| p.exists())
            .and_then(|p| std::fs::read_to_string(p).ok())
    };
    let started = std::time::Instant::now();
    while beat().is_none() {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(20),
            "qjs never beat"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    tree.kill().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let then = beat();
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(beat(), then, "qjs still runs after its tree was ended");
}
