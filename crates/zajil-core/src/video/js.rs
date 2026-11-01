//! The JavaScript runtime yt-dlp may use (deno, or our own sandbox), how it
//! is found, and what its sandbox reports.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::tools::VerifiedTool;

/// The tools of one run, checked right before it ([`Toolbox::verified`]),
/// and kept until its whole tree has ended: on Windows they stay locked
/// while yt-dlp may start ffmpeg or ffprobe (review of the video runner:
/// they were checked once, when the runner was made).
pub(super) struct Checked {
    pub(super) yt_dlp: VerifiedTool,
    pub(super) ffmpeg: VerifiedTool,
    pub(super) _ffprobe: VerifiedTool,
    /// `qjs` and its pinned SHA-256, when the runtime is our QuickJS-NG.
    pub(super) qjs: Option<(VerifiedTool, String)>,
}

impl Checked {
    /// The folder of our ffmpeg and ffprobe, for `--ffmpeg-location`.
    pub(super) fn ffmpeg_dir(&self) -> PathBuf {
        self.ffmpeg
            .path()
            .parent()
            .expect("a tool has a folder")
            .to_owned()
    }
}

/// The JavaScript runtime yt-dlp runs YouTube's challenges with (ADR-0009,
/// owner's order): Deno if installed, else Node, else our QuickJS-NG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsRuntime {
    /// Deno 2.3 or later, found on the user's `PATH`: yt-dlp runs it with
    /// no permission at all (Deno's own sandbox).
    Deno(PathBuf),
    /// Node 22 or later, found on the user's `PATH`: yt-dlp runs it with
    /// Node's permission model.
    Node(PathBuf),
    /// Our pinned QuickJS-NG, which has no sandbox of its own: yt-dlp runs
    /// `zajil-jsbox`, which runs `qjs` in ours, with limits.
    /// The checked `qjs` and its pinned SHA-256 are given to the box for
    /// each run ([`Toolbox::verified`]), and it checks them again right
    /// before running it.
    QuickJs { jsbox: PathBuf },
}

/// The oldest Deno and Node that yt-dlp accepts.
const DENO_MIN: (u64, u64) = (2, 3);
const NODE_MIN: (u64, u64) = (22, 0);

/// Every `name` (with `.exe` on Windows) in `path`'s folders, in order:
/// one that is not usable must not hide a usable one further on (review
/// of the YouTube PR). Relative folders are never searched.
fn on_path<'a>(path: &'a std::ffi::OsStr, name: &str) -> impl Iterator<Item = PathBuf> + 'a {
    let suffix = zajil_platform::target::current().map_or("", |t| t.exe_suffix());
    let file_name = format!("{name}{suffix}");
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .map(move |dir| dir.join(&file_name))
        .filter(|file| file.is_file())
}

/// The major and minor version a runtime prints for `--version` (`deno
/// 2.5.6 (...)`, `v22.11.0`), within a few seconds; or none.
fn version_of(program: &Path) -> Option<(u64, u64)> {
    let command = || {
        let mut command = Command::new(program);
        command
            .arg("--version")
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
        command
    };
    // A file that some process holds open for writing cannot be run:
    // «text file busy» on Linux (ETXTBSY), a sharing violation on Windows
    // (error 32). A file just written (a runtime just installed, a test's
    // stand-in) can be held that way for a moment: on Linux by any process
    // that forks meanwhile, until its child runs its own program (#180); on
    // Windows by whoever wrote or scans it. Tried again a few times, 20 ms
    // apart; any other failure is final.
    let busy = |err: &std::io::Error| {
        err.kind() == std::io::ErrorKind::ExecutableFileBusy
            || (cfg!(windows) && err.raw_os_error() == Some(32))
    };
    let mut tries = 0;
    let mut tree = loop {
        match zajil_platform::process::spawn(command()) {
            Ok(tree) => break tree,
            Err(err) if busy(&err) && tries < 25 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    };
    let mut stdout = tree.take_stdout()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ =
            std::io::Read::read_to_string(&mut std::io::Read::take(&mut stdout, 4096), &mut text);
        text
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while tree.try_wait().ok()?.is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = tree.kill();
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _ = tree.kill();
    parse_version(&reader.join().ok()?)
}

/// The first `major.minor` of `text`'s first line, after `deno ` or `v`.
fn parse_version(text: &str) -> Option<(u64, u64)> {
    let line = text.lines().next()?.trim();
    let rest = line
        .strip_prefix("deno ")
        .or_else(|| line.strip_prefix('v'))?;
    let mut parts = rest.split(|c: char| !c.is_ascii_digit());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// The real file behind `file` (through any link: Homebrew, snap and
/// Debian's alternatives install runtimes as links), if no other user can
/// change it, or put another in its place through any folder above it
/// (ADR-0009, section 3: a runtime the user installed has no pinned hash;
/// review of the YouTube PR: only its own folder was checked).
fn only_ours(file: &Path) -> Option<PathBuf> {
    let real = file.canonicalize().ok()?;
    let ok = |result: io::Result<bool>| result.unwrap_or(false);
    let folders_ok = real
        .ancestors()
        .skip(1)
        .all(|dir| ok(zajil_platform::fs::no_other_user_can_replace_in(dir)));
    (real.is_file() && ok(zajil_platform::fs::only_user_or_root_can_change(&real)) && folders_ok)
        // Without Windows' `\\?\` prefix, which yt-dlp is not known to
        // run a program from (checked above in its full form).
        .then(|| dunce::simplified(&real).to_path_buf())
}

/// The runtime for YouTube's challenges, in the owner's order: Deno 2.3+
/// on the user's `PATH`, else Node 22+, else our QuickJS-NG through
/// `jsbox` (given only when `qjs` is installed), else none. One that
/// another user could change is passed over. `always_quickjs` is the setting «استعمل QuickJS-NG
/// دائمًا»: Deno and Node are not even looked for. Runs `--version` of
/// what it finds: blocking.
pub fn find_js_runtime(
    path: Option<&std::ffi::OsStr>,
    jsbox: Option<PathBuf>,
    always_quickjs: bool,
) -> Option<JsRuntime> {
    if let Some(path) = path.filter(|_| !always_quickjs) {
        for (name, min, make) in [
            (
                "deno",
                DENO_MIN,
                JsRuntime::Deno as fn(PathBuf) -> JsRuntime,
            ),
            ("node", NODE_MIN, JsRuntime::Node),
        ] {
            let usable = on_path(path, name)
                .filter_map(|file| only_ours(&file))
                .find(|found| version_of(found).is_some_and(|v| v >= min));
            if let Some(found) = usable {
                return Some(make(found));
            }
        }
    }
    jsbox.map(|jsbox| JsRuntime::QuickJs { jsbox })
}

/// Where `zajil-jsbox` is: next to `program` (Zajil's own executable).
pub fn jsbox_beside(program: &Path) -> Option<PathBuf> {
    let suffix = zajil_platform::target::current().map_or("", |t| t.exe_suffix());
    let jsbox = program.parent()?.join(format!("zajil-jsbox{suffix}"));
    jsbox.is_file().then_some(jsbox)
}

/// What the sandbox of `zajil-jsbox` held, when it did not hold everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsSandbox {
    /// Starting any other program was refused.
    pub programs: bool,
    /// Writing outside its folder was refused.
    pub files: bool,
    /// The network was refused.
    pub network: bool,
}

impl JsSandbox {
    /// What held in both: the weaker of the two.
    pub(super) fn weakest(self, other: Option<JsSandbox>) -> JsSandbox {
        match other {
            None => self,
            Some(o) => JsSandbox {
                programs: self.programs && o.programs,
                files: self.files && o.files,
                network: self.network && o.network,
            },
        }
    }
}

impl From<zajil_platform::sandbox::Confinement> for JsSandbox {
    fn from(c: zajil_platform::sandbox::Confinement) -> Self {
        JsSandbox {
            programs: c.programs,
            files: c.files,
            network: c.network,
        }
    }
}

/// What the sandbox held in a run that may have used `zajil-jsbox`: never
/// more than this system supports, whatever the reports say (a program the
/// system does not confine could write its own). With no report, the box
/// never ran, unless the system cannot keep the box from the report folder:
/// then the reports may have been removed, and only the system's answer is
/// left (review of the YouTube PR).
pub(super) fn js_held(reports: Option<JsSandbox>, system: JsSandbox) -> Option<JsSandbox> {
    match reports {
        Some(reports) => Some(reports.weakest(Some(system))),
        None if !system.files => Some(system),
        None => None,
    }
}

/// The reports `zajil-jsbox` wrote in `dir`: the weakest, or none if it
/// never ran.
pub(super) fn js_reports(dir: &Path) -> Option<JsSandbox> {
    let mut weakest: Option<JsSandbox> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        // A short line; never through a link.
        let mut text = String::new();
        let read = zajil_platform::fs::open_regular_nofollow(&entry.path(), false)
            .and_then(|f| f.take(256).read_to_string(&mut text));
        if read.is_err() {
            continue;
        }
        let says = |name: &str| text.split_whitespace().any(|w| w == format!("{name}=true"));
        let this = JsSandbox {
            programs: says("programs"),
            files: says("files"),
            network: says("network"),
        };
        weakest = Some(this.weakest(weakest));
    }
    weakest
}

/// The folder of a run where `zajil-jsbox` reports.
pub(super) const JS_REPORTS: &str = "js-report";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_versions_are_read_from_their_first_line() {
        assert_eq!(
            parse_version("deno 2.5.6 (stable, release, x86_64)\nv8 14"),
            Some((2, 5))
        );
        assert_eq!(parse_version("v22.11.0\n"), Some((22, 11)));
        assert_eq!(parse_version("v8.1"), Some((8, 1)));
        for bad in ["", "node 22.1.0", "deno x.y", "v22", "Deno 2.5.0"] {
            assert_eq!(parse_version(bad), None, "{bad:?}");
        }
        assert!((2, 5) >= DENO_MIN && (2, 2) < DENO_MIN && (1, 46) < DENO_MIN);
        assert!((22, 0) >= NODE_MIN && (21, 9) < NODE_MIN);
    }

    #[test]
    fn a_report_never_says_more_than_the_system_supports() {
        let held = |programs, files, network| JsSandbox {
            programs,
            files,
            network,
        };
        let all = held(true, true, true);
        // A box that is not confined could write "all held".
        assert_eq!(
            js_held(Some(all), held(false, false, false)),
            Some(held(false, false, false))
        );
        assert_eq!(
            js_held(Some(all), held(true, true, false)),
            Some(held(true, true, false))
        );
        assert_eq!(
            js_held(Some(held(true, false, true)), all),
            Some(held(true, false, true))
        );
        // No report: it never ran, unless it could have removed its report.
        assert_eq!(js_held(None, all), None);
        assert_eq!(
            js_held(None, held(true, false, true)),
            Some(held(true, false, true))
        );
    }

    #[test]
    fn the_weakest_sandbox_report_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let held = |programs, files, network| {
            Some(JsSandbox {
                programs,
                files,
                network,
            })
        };
        assert_eq!(js_reports(&dir.path().join("none")), None);
        assert_eq!(js_reports(dir.path()), None);
        let all = "programs=true files=true network=true\n";
        std::fs::write(dir.path().join("a.txt"), all).unwrap();
        assert_eq!(js_reports(dir.path()), held(true, true, true));
        let some = "programs=true files=true network=false\n";
        std::fs::write(dir.path().join("b.txt"), some).unwrap();
        assert_eq!(js_reports(dir.path()), held(true, true, false));
        std::fs::write(dir.path().join("c.txt"), "garbage").unwrap();
        assert_eq!(js_reports(dir.path()), held(false, false, false));
    }
}
