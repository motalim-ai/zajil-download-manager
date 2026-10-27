//! The programs Zajil runs for video downloads (ADR-0009): yt-dlp, our own
//! LGPL build of ffmpeg and ffprobe, and QuickJS-NG for YouTube's
//! JavaScript challenges (run only through `zajil-jsbox`, in a sandbox).
//!
//! None of them ships in the installer. Each is downloaded the first time
//! the user downloads a video, after they agree, from the release named
//! here, and kept only if its SHA-256 is the one written in this file.
//! Before every run its SHA-256 is checked again, and so is that no other
//! user can change it or its folder. There is no other way to get or
//! update a tool: never `yt-dlp -U`, never a copy found on the system.
//!
//! On macOS and Windows yt-dlp is the official zip of its "onedir" build
//! (owner's decision, 2026-10-27: the one-file build unpacks itself into a
//! new temporary folder at every start, which the system then checks, up
//! to 21 s each time on a Mac). The zip is kept only if its SHA-256 is the
//! pinned one; it is then unpacked once, safely, into the version's folder,
//! and what it holds must have the pinned "tree" SHA-256 ([`tree_sha256`]).
//! Before every run every file of that folder is checked again, and
//! nothing may be missing or added.
//!
//! The interface asks for consent; this module never downloads by itself.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use zajil_platform::target::{Arch, Os, Target};

use crate::engine::{Engine, Error as EngineError, Event, Stop};

/// A program Zajil runs for video downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    YtDlp,
    Ffmpeg,
    Ffprobe,
    /// QuickJS-NG's `qjs`, used when neither Deno nor Node is installed.
    QuickJs,
}

impl std::fmt::Display for Tool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Tool::YtDlp => "yt-dlp",
            Tool::Ffmpeg => "ffmpeg",
            Tool::Ffprobe => "ffprobe",
            Tool::QuickJs => "QuickJS-NG",
        })
    }
}

impl Tool {
    pub const ALL: [Tool; 4] = [Tool::YtDlp, Tool::Ffmpeg, Tool::Ffprobe, Tool::QuickJs];

    /// The name the program is saved under (with `.exe` on Windows). FFmpeg's
    /// files keep FFmpeg's own names (LGPL checklist, item 16).
    pub fn file_name(self, target: Target) -> String {
        let base = match self {
            Tool::YtDlp => "yt-dlp",
            Tool::Ffmpeg => "ffmpeg",
            Tool::Ffprobe => "ffprobe",
            Tool::QuickJs => "qjs",
        };
        format!("{base}{}", target.exe_suffix())
    }

    /// The folder of its version under the tools folder: ffmpeg and ffprobe
    /// share one, so yt-dlp finds both through `--ffmpeg-location`.
    fn folder(self) -> &'static str {
        match self {
            Tool::YtDlp => "yt-dlp",
            Tool::Ffmpeg | Tool::Ffprobe => "ffmpeg",
            Tool::QuickJs => "quickjs",
        }
    }
}

/// Where one tool comes from, and what it must be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pinned {
    /// Its version, also the name of its folder.
    pub version: String,
    /// The release file, over HTTPS.
    pub url: String,
    /// The SHA-256 of that file, lower-case hex.
    pub sha256: String,
    /// When the release file is a zip of the program and its files.
    pub archive: Option<Archive>,
}

/// A tool released as a zip: its program at the top of the zip, and what
/// the whole of it must be once unpacked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    /// The folder it is unpacked into, in its version's folder.
    pub folder: String,
    /// The program, at the top of that folder (`yt-dlp_macos`, `yt-dlp.exe`).
    pub program: String,
    /// [`tree_sha256`] of the unpacked files.
    pub tree_sha256: String,
}

/// The pinned yt-dlp (`tools/yt-dlp/pins.txt`): its stable version, then
/// `<os>-<arch> <release file> <sha256>` lines, read strictly
/// ([`parse_pins`]; a test reads the file of this build). A newer version
/// comes with a new Zajil release, after its tests pass (ADR-0009, section
/// 7).
const YT_DLP_PINS: &str = include_str!("../../../tools/yt-dlp/pins.txt");
/// The pinned QuickJS-NG (`tools/quickjs/pins.txt`), in the same form.
const QUICKJS_PINS: &str = include_str!("../../../tools/quickjs/pins.txt");
/// Zajil's own FFmpeg build (`tools/ffmpeg/pins-ffmpeg.txt` and
/// `pins-ffprobe.txt`), in the same form, from the `tools-ffmpeg-<version>`
/// release of this repository. Both name one version: they share a folder.
const FFMPEG_PINS: &str = include_str!("../../../tools/ffmpeg/pins-ffmpeg.txt");
const FFPROBE_PINS: &str = include_str!("../../../tools/ffmpeg/pins-ffprobe.txt");

/// A pins file: one version, and the file and SHA-256 of each target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinsFile {
    pub version: String,
    /// Each target once.
    pub files: Vec<PinLine>,
}

/// One target's line of a pins file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinLine {
    /// `<os>-<arch>`.
    pub target: String,
    /// The release file.
    pub file: String,
    pub sha256: String,
    /// For a zip: its program and the SHA-256 of what it holds.
    pub archive: Option<(String, String)>,
}

/// Reads a pins file strictly (review of the video tools): comments (`#`)
/// and empty lines, one `version <version>` line, then per line: a known
/// target, once; a plain file name; a lower-case SHA-256; and for a
/// `.zip` file only, two more: its program (a plain name) and the
/// lower-case [`tree_sha256`] of what it holds. The version and names are
/// plain names (letters, digits, `-`, `_`, `.`, never `.` or `..`): the
/// version names a folder.
pub fn parse_pins(text: &str) -> Result<PinsFile, String> {
    let mut version = None;
    let mut files: Vec<PinLine> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let at = || format!("line {}", n + 1);
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["version", v] => {
                if version.is_some() {
                    return Err(format!("{}: a second version", at()));
                }
                if !is_plain_name(v) {
                    return Err(format!("{}: the version is not a plain name", at()));
                }
                version = Some((*v).to_owned());
            }
            [target, file, sha256, rest @ ..] if rest.is_empty() || rest.len() == 2 => {
                if !KNOWN_TARGETS.contains(target) {
                    return Err(format!("{}: unknown target {target:?}", at()));
                }
                if files.iter().any(|l| l.target == *target) {
                    return Err(format!("{}: {target} twice", at()));
                }
                if !is_plain_name(file) {
                    return Err(format!("{}: the file is not a plain name", at()));
                }
                if !is_sha256(sha256) {
                    return Err(format!("{}: not a lower-case SHA-256", at()));
                }
                let zip = file.strip_suffix(".zip").filter(|stem| is_plain_name(stem));
                let archive = match (rest, zip) {
                    ([], None) => None,
                    ([program, tree], Some(_)) => {
                        if !is_plain_name(program) {
                            return Err(format!("{}: the program is not a plain name", at()));
                        }
                        if !is_sha256(tree) {
                            return Err(format!("{}: not a lower-case tree SHA-256", at()));
                        }
                        Some(((*program).to_owned(), (*tree).to_owned()))
                    }
                    ([], Some(_)) => {
                        return Err(format!("{}: a zip without its program and tree", at()));
                    }
                    _ => {
                        return Err(format!(
                            "{}: a program and tree for a file that is not a zip",
                            at()
                        ));
                    }
                };
                files.push(PinLine {
                    target: (*target).to_owned(),
                    file: (*file).to_owned(),
                    sha256: (*sha256).to_owned(),
                    archive,
                });
            }
            _ => {
                return Err(format!(
                    "{}: not `version <v>`, `<target> <file> <sha256>` nor `<target> <file>.zip <sha256> <program> <tree sha256>`",
                    at()
                ));
            }
        }
    }
    let version = version.ok_or("no version line")?;
    Ok(PinsFile { version, files })
}

const KNOWN_TARGETS: [&str; 6] = [
    "windows-x86_64",
    "windows-aarch64",
    "macos-x86_64",
    "macos-aarch64",
    "linux-x86_64",
    "linux-aarch64",
];

/// A lower-case SHA-256 in hex.
fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A file or folder name with nothing that could leave its folder.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// This build's yt-dlp pins (checked by a test, so never wrong here).
fn yt_dlp_pins() -> &'static PinsFile {
    static PINS: std::sync::OnceLock<PinsFile> = std::sync::OnceLock::new();
    PINS.get_or_init(|| parse_pins(YT_DLP_PINS).expect("tools/yt-dlp/pins.txt is valid"))
}

/// This build's QuickJS-NG pins (checked by a test, like yt-dlp's).
fn quickjs_pins() -> &'static PinsFile {
    static PINS: std::sync::OnceLock<PinsFile> = std::sync::OnceLock::new();
    PINS.get_or_init(|| parse_pins(QUICKJS_PINS).expect("tools/quickjs/pins.txt is valid"))
}

/// This build's ffmpeg and ffprobe pins (checked by a test, like yt-dlp's).
fn ffmpeg_pins() -> &'static PinsFile {
    static PINS: std::sync::OnceLock<PinsFile> = std::sync::OnceLock::new();
    PINS.get_or_init(|| parse_pins(FFMPEG_PINS).expect("tools/ffmpeg/pins-ffmpeg.txt is valid"))
}

fn ffprobe_pins() -> &'static PinsFile {
    static PINS: std::sync::OnceLock<PinsFile> = std::sync::OnceLock::new();
    PINS.get_or_init(|| parse_pins(FFPROBE_PINS).expect("tools/ffmpeg/pins-ffprobe.txt is valid"))
}

/// The pinned version of yt-dlp.
pub fn yt_dlp_version() -> &'static str {
    &yt_dlp_pins().version
}

/// The name of `target` in pins.txt.
fn target_name(target: Target) -> String {
    let os = match target.os {
        Os::Windows => "windows",
        Os::Macos => "macos",
        Os::Linux => "linux",
    };
    let arch = match target.arch {
        Arch::X86_64 => "x86_64",
        Arch::Aarch64 => "aarch64",
    };
    format!("{os}-{arch}")
}

/// The pinned file of `pins` for `target`, if it has one, from the GitHub
/// releases of `repository`, under the tag `<tag_prefix><version>`.
fn from_pins(
    pins: &PinsFile,
    repository: &str,
    tag_prefix: &str,
    target: Target,
) -> Option<Pinned> {
    let wanted = target_name(target);
    let version = &pins.version;
    pins.files.iter().find(|l| l.target == wanted).map(|line| {
        let file = &line.file;
        Pinned {
            version: version.clone(),
            url: format!(
                "https://github.com/{repository}/releases/download/{tag_prefix}{version}/{file}"
            ),
            sha256: line.sha256.clone(),
            archive: line.archive.as_ref().map(|(program, tree)| Archive {
                folder: file.strip_suffix(".zip").unwrap_or(file).to_owned(),
                program: program.clone(),
                tree_sha256: tree.clone(),
            }),
        }
    })
}

/// Where Zajil's FFmpeg build is released (.github/workflows/ffmpeg-release.yml).
const FFMPEG_REPOSITORY: &str = "motalim-ai/zajil-download-manager";

/// What a [`Toolbox`] downloads: a tool and a target to the pinned file,
/// or `None` while that file is not released yet.
pub type Pins = dyn Fn(Tool, Target) -> Option<Pinned> + Send + Sync;

/// The files pinned in this build of Zajil.
pub fn pinned(tool: Tool, target: Target) -> Option<Pinned> {
    match tool {
        Tool::YtDlp => from_pins(yt_dlp_pins(), "yt-dlp/yt-dlp", "", target),
        Tool::QuickJs => from_pins(quickjs_pins(), "quickjs-ng/quickjs", "", target),
        Tool::Ffmpeg => from_pins(ffmpeg_pins(), FFMPEG_REPOSITORY, "tools-ffmpeg-", target),
        Tool::Ffprobe => from_pins(ffprobe_pins(), FFMPEG_REPOSITORY, "tools-ffmpeg-", target),
    }
}

/// Why a tool cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("{0} has no released file for this system yet")]
    NotReleased(Tool),
    #[error("this system or processor has no video tools")]
    UnknownTarget,
    #[error("{0} is not downloaded yet")]
    Missing(Tool),
    /// The file is not the pinned one: refused, and removed if it was a
    /// new download.
    #[error("{tool} is not the expected file (SHA-256 {actual}, expected {expected})")]
    Mismatch {
        tool: Tool,
        actual: String,
        expected: String,
    },
    /// Its zip, or what was unpacked from it, is not as pinned: refused,
    /// and removed if it was a new download.
    #[error("{0} is not as pinned: {1}")]
    BadArchive(Tool, String),
    #[error("{} can be changed by another user, so it is not run", .0.display())]
    NotPrivate(PathBuf),
    /// A pin that is not a plain version and an HTTPS file (only a test's
    /// own pins could be).
    #[error("{0:?} has an invalid pin")]
    InvalidPin(Tool),
    #[error("downloading the tool failed: {0}")]
    Download(#[from] EngineError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The tools folder of one user, `<data dir>/tools`.
pub struct Toolbox {
    root: PathBuf,
    target: Option<Target>,
    pins: Box<Pins>,
}

impl Toolbox {
    /// The tools of this build, in `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Toolbox {
            root: root.into(),
            target: zajil_platform::target::current(),
            pins: Box::new(pinned),
        }
    }

    /// With other pinned files: tests serve their own.
    #[cfg(feature = "test-support")]
    pub fn with_pins(root: impl Into<PathBuf>, pins: Box<Pins>) -> Self {
        Toolbox {
            pins,
            ..Toolbox::new(root)
        }
    }

    fn pin(&self, tool: Tool) -> Result<(Pinned, Target), ToolError> {
        let target = self.target.ok_or(ToolError::UnknownTarget)?;
        let pin = (self.pins)(tool, target).ok_or(ToolError::NotReleased(tool))?;
        let archive_ok = pin.archive.as_ref().is_none_or(|a| {
            is_plain_name(&a.folder) && is_plain_name(&a.program) && is_sha256(&a.tree_sha256)
        });
        if !is_plain_name(&pin.version) || !pin.url.starts_with("https://") || !archive_ok {
            return Err(ToolError::InvalidPin(tool));
        }
        Ok((pin, target))
    }

    /// The pinned SHA-256 of `tool` for this system (lower-case hex).
    pub fn sha256(&self, tool: Tool) -> Result<String, ToolError> {
        Ok(self.pin(tool)?.0.sha256)
    }

    /// Where `tool` is, or would be, kept: for a zip, its program in the
    /// folder it is unpacked into.
    pub fn path(&self, tool: Tool) -> Result<PathBuf, ToolError> {
        let (pin, target) = self.pin(tool)?;
        let version = self.root.join(tool.folder()).join(&pin.version);
        Ok(match &pin.archive {
            Some(archive) => version.join(&archive.folder).join(&archive.program),
            None => version.join(tool.file_name(target)),
        })
    }

    /// Whether this user has `tool` of another version or build than the
    /// pinned one (yt-dlp's one-file build, now that its zip is pinned), and
    /// not the pinned one: they agreed to it then, so it is brought up to
    /// date without asking again (owner's decision, 2026-10-27). Told by
    /// the folders alone, without hashing anything (review of #278): a
    /// file of another version's folder, or in the version's folder
    /// something else than the pinned zip's folder, while the pinned
    /// program is not there. A tool that is there but changed is not
    /// "earlier": it is refused before it runs, as before. Blocking.
    pub fn has_an_earlier_one(&self, tool: Tool) -> bool {
        let (Ok((pin, _)), Ok(current)) = (self.pin(tool), self.path(tool)) else {
            return false;
        };
        if std::fs::symlink_metadata(&current).is_ok() {
            return false;
        }
        let folder = self.root.join(tool.folder());
        let holds_a_file = |dir: &Path, skip: Option<&str>| {
            fn any_file(dir: &Path, depth: usize) -> bool {
                std::fs::read_dir(dir).is_ok_and(|entries| {
                    entries.flatten().any(|e| match e.file_type() {
                        Ok(t) if t.is_dir() => depth < 4 && any_file(&e.path(), depth + 1),
                        Ok(_) => true,
                        Err(_) => false,
                    })
                })
            }
            std::fs::read_dir(dir).is_ok_and(|entries| {
                entries.flatten().any(|e| {
                    skip.is_none_or(|s| e.file_name() != std::ffi::OsStr::new(s))
                        && match e.file_type() {
                            Ok(t) if t.is_dir() => any_file(&e.path(), 1),
                            Ok(_) => true,
                            Err(_) => false,
                        }
                })
            })
        };
        let other_versions = holds_a_file(&folder, Some(&pin.version));
        let other_build = pin
            .archive
            .as_ref()
            .is_some_and(|archive| holds_a_file(&folder.join(&pin.version), Some(&archive.folder)));
        other_versions || other_build
    }

    /// Whether `tool` is there and is the pinned file. Reads and hashes it,
    /// so call it from a blocking task.
    pub fn is_installed(&self, tool: Tool) -> bool {
        self.verified_blocking(tool).is_ok()
    }

    /// `tool`, checked right before it runs: the pinned file, which no
    /// other user can change, in folders no other user can change, from
    /// the tools folder down. Keep the [`VerifiedTool`] until the program
    /// has started: on Windows the file stays locked while it lives.
    pub async fn verified(&self, tool: Tool) -> Result<VerifiedTool, ToolError> {
        let (pin, path) = (self.pin(tool)?.0, self.path(tool)?);
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || check_installed(tool, &pin, &root, &path))
            .await
            .map_err(io::Error::other)?
    }

    fn verified_blocking(&self, tool: Tool) -> Result<VerifiedTool, ToolError> {
        let (pin, path) = (self.pin(tool)?.0, self.path(tool)?);
        check_installed(tool, &pin, &self.root, &path)
    }

    /// Downloads `tool` with `engine` (TLS checked as for every download)
    /// and keeps it only if it is the pinned file. Only after the user
    /// agreed. Nothing but the checked tool is left in the tools folder,
    /// also when this fails or is stopped.
    pub async fn install(
        &self,
        engine: &Engine,
        tool: Tool,
        stop: &Stop,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<PathBuf, ToolError> {
        let (pin, _) = self.pin(tool)?;
        let dest = self.path(tool)?;
        let root = self.root.clone();
        // A private folder of its own for this download: the engine's part
        // and state files and the raw file stay out of the tools' folders.
        let staging = tokio::task::spawn_blocking(move || -> io::Result<Staging> {
            zajil_platform::fs::private_dir(&root)?;
            Staging::new(&root)
        })
        .await
        .map_err(io::Error::other)??;
        let finished = match engine
            .download_until(&pin.url, staging.path(), stop, on_event)
            .await
        {
            Ok(finished) => finished,
            Err(err) => {
                // Gone before the error is returned, so a caller that looks
                // finds nothing (only a dropped future leaves the removal
                // to Drop).
                let _ = tokio::task::spawn_blocking(move || staging.remove()).await;
                return Err(err.into());
            }
        };
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let result = match &pin.archive {
                Some(archive) => keep_archive_if_pinned(
                    tool,
                    &pin,
                    archive,
                    &finished.file,
                    staging.path(),
                    &dest,
                ),
                None => keep_if_pinned(tool, &pin, &finished.file, staging.path(), &dest),
            }
            .and_then(|kept| {
                unmark_folders(&root, kept.parent().expect("a tool path has a folder"))?;
                Ok(kept)
            });
            staging.remove();
            if result.is_ok() {
                remove_earlier(&root, tool, &pin);
            }
            result
        })
        .await
        .map_err(io::Error::other)?
    }

    /// Removes the download folders a crash or a stopped download whose
    /// last writes outlived it may have left (`.download-*`). At program
    /// start, before any install; blocking.
    pub fn remove_leftovers(&self) -> io::Result<()> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err),
        };
        for entry in entries {
            let entry = entry?;
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(".download-")
            {
                // remove_dir_all removes a link itself, never what it
                // points to.
                std::fs::remove_dir_all(entry.path())?;
            }
        }
        Ok(())
    }

    /// Removes every tool, for the setting that deletes them. Blocking.
    pub fn remove_all(&self) -> io::Result<()> {
        match std::fs::remove_dir_all(&self.root) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        }
    }
}

/// A download folder under the tools folder. [`Staging::remove`] removes it
/// with what is in it; a `Staging` dropped without that (an install future
/// dropped mid-way) removes it on a blocking thread.
struct Staging(Option<PathBuf>);

impl Staging {
    fn new(root: &Path) -> io::Result<Self> {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).map_err(io::Error::other)?;
        let dir = root.join(format!(".download-{}", hex(&bytes)));
        zajil_platform::fs::private_dir(&dir)?;
        Ok(Staging(Some(dir)))
    }

    fn path(&self) -> &Path {
        self.0.as_deref().expect("not removed yet")
    }

    /// Removes the folder now, again for a while if a scanner still holds
    /// a file in it. Blocking.
    fn remove(mut self) {
        if let Some(dir) = self.0.take() {
            let _ = zajil_platform::fs::remove_dir_all_retrying(&dir);
        }
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let Some(dir) = self.0.take() else { return };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => drop(runtime.spawn_blocking(move || std::fs::remove_dir_all(dir))),
            Err(_) => {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

/// Copies the downloaded file to `dest` through a new file only the user
/// can access, hashing what it copies, and keeps it only if the hash is the
/// pinned one. A copy, not a rename: the new file carries no mark from the
/// internet (ADR-0009, section 2; on macOS once [`unmark`] took the one
/// every file of a marked Zajil gets) and is not the engine's file. The
/// copy is made in `staging`, this install's own folder on the same volume,
/// so two installs at once never share a file (review of the video tools).
fn keep_if_pinned(
    tool: Tool,
    pin: &Pinned,
    from: &Path,
    staging: &Path,
    dest: &Path,
) -> Result<PathBuf, ToolError> {
    let dir = dest.parent().expect("a tool path has a folder");
    zajil_platform::fs::private_dir(dir)?;
    let mut source = zajil_platform::fs::open_regular_nofollow(from, false)?;
    let new = staging.join("tool.new");
    let mut out = zajil_platform::fs::create_private(&new)?;
    let copied = copy_hashing(&mut source, &mut out).and_then(|actual| {
        out.sync_all()?;
        Ok(actual)
    });
    let actual = match copied {
        Ok(actual) => actual,
        Err(err) => {
            drop(out);
            let _ = std::fs::remove_file(&new);
            return Err(err.into());
        }
    };
    if actual != pin.sha256 {
        drop(out);
        let _ = std::fs::remove_file(&new);
        return Err(ToolError::Mismatch {
            tool,
            actual,
            expected: pin.sha256.clone(),
        });
    }
    unmark(&new)?;
    zajil_platform::fs::make_executable(&out)?;
    drop(out);
    zajil_platform::fs::replace_file(&new, dest)?;
    zajil_platform::fs::sync_dir(dir)?;
    Ok(dest.to_owned())
}

/// The most entries a tool's zip may hold, and the most it may unpack to:
/// far above yt-dlp's (under 200 entries and 130 MB), far below a zip bomb.
const MAX_ENTRIES: usize = 4096;
const MAX_UNPACKED: u64 = 1024 * 1024 * 1024;
/// The deepest path, and the longest, a tool's zip may hold.
const MAX_DEPTH: usize = 16;
const MAX_PATH: usize = 512;

/// The SHA-256 of a tool's unpacked files: one line per file, sorted by its
/// path inside the folder (`/` between names), `<path>\0<file's SHA-256>\n`,
/// hashed. Folders count only through their files. Pinned for each zip in
/// pins.txt (`cargo run -p zajil-core --example tool-tree -- <zip>
/// <program>` gives it).
pub fn tree_sha256(files: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut hasher = Sha256::new();
    for (path, sha256) in sorted {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(sha256.as_bytes());
        hasher.update(b"\n");
    }
    hex(&hasher.finalize())
}

/// A path inside a tool's zip that can only land inside its folder: names
/// of letters, digits, `.`, `_` and `-`, never `.` or `..`, never ending in
/// `.` (Windows would drop it), joined by `/`, not too deep or too long.
fn zip_path(name: &str) -> Option<Vec<&str>> {
    let name = name.strip_suffix('/').unwrap_or(name);
    if name.is_empty() || name.len() > MAX_PATH {
        return None;
    }
    let parts: Vec<&str> = name.split('/').collect();
    // Windows opens these as devices, whatever comes after a dot.
    let device = |part: &str| {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit())
    };
    let good = parts.len() <= MAX_DEPTH
        && parts.iter().all(|part| {
            !part.is_empty()
                && *part != "."
                && *part != ".."
                && !part.ends_with('.')
                && !device(part)
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        });
    good.then_some(parts)
}

/// Unpacks the zip `file` into the new folder `into` (it must not exist),
/// as only this user can access: only plain folders and files (no link, no
/// other kind), each path checked ([`zip_path`]), within the limits above;
/// each file written new, hashed as it is written, checked by the zip's
/// CRC-32, and made executable when the zip says so or it is `program`.
/// Returns the [`tree_sha256`] of what it unpacked. `program` must be a
/// file at its top. Blocking.
///
/// Only a zip whose own SHA-256 was the pinned one reaches this in Zajil;
/// the checks are there all the same.
pub fn unpack_zip(file: File, into: &Path, program: &str) -> Result<String, String> {
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a zip: {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("{} entries, more than {MAX_ENTRIES}", zip.len()));
    }
    zajil_platform::fs::private_dir(into).map_err(|e| e.to_string())?;
    let mut files: Vec<(String, String)> = Vec::new();
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("entry {i}: {e}"))?;
        // The name as the zip holds it, checked here, not one the reader
        // cleaned up.
        let name = std::str::from_utf8(entry.name_raw())
            .map_err(|_| format!("entry {i}: a name that is not UTF-8"))?
            .to_owned();
        let parts = zip_path(&name).ok_or_else(|| format!("{name:?} is not a plain path"))?;
        let mode = entry.unix_mode().unwrap_or(0);
        let kind = mode & 0o170_000;
        if entry.is_dir() {
            if kind != 0 && kind != 0o040_000 {
                return Err(format!("{name:?} is not a plain folder"));
            }
            let mut dir = into.to_path_buf();
            dir.extend(&parts);
            zajil_platform::fs::private_dir(&dir).map_err(|e| e.to_string())?;
            continue;
        }
        // A link (or anything else) is refused: only plain files.
        if kind != 0 && kind != 0o100_000 {
            return Err(format!("{name:?} is not a plain file"));
        }
        total = total.saturating_add(entry.size());
        if total > MAX_UNPACKED {
            return Err(format!("it unpacks to more than {MAX_UNPACKED} bytes"));
        }
        let mut path = into.to_path_buf();
        path.extend(&parts);
        if parts.len() > 1 {
            zajil_platform::fs::private_dir(path.parent().expect("a file in a folder"))
                .map_err(|e| e.to_string())?;
        }
        let mut out =
            zajil_platform::fs::create_private(&path).map_err(|e| format!("{name:?}: {e}"))?;
        // No more than the entry says it holds; the reader checks its
        // CRC-32 at its end.
        let size = entry.size();
        let mut limited = (&mut entry).take(size);
        let sha256 =
            copy_hashing_from(&mut limited, &mut out).map_err(|e| format!("{name:?}: {e}"))?;
        let mut rest = [0u8; 1];
        if entry
            .read(&mut rest)
            .map_err(|e| format!("{name:?}: {e}"))?
            != 0
        {
            return Err(format!("{name:?} holds more than it says"));
        }
        out.sync_all().map_err(|e| e.to_string())?;
        let top_program = parts.len() == 1 && parts[0] == program;
        if top_program || mode & 0o111 != 0 {
            zajil_platform::fs::make_executable(&out).map_err(|e| e.to_string())?;
        }
        files.push((parts.join("/"), sha256));
    }
    if !files.iter().any(|(path, _)| path == program) {
        return Err(format!("no {program} at its top"));
    }
    Ok(tree_sha256(&files))
}

/// Keeps a tool released as a zip: the downloaded zip must have the pinned
/// SHA-256 (checked before it is read as a zip), then it is unpacked into
/// this install's own folder, what it holds must have the pinned tree
/// SHA-256, and only then the folder is put in place (replacing a broken
/// or earlier one). Nothing of a refused zip is left.
fn keep_archive_if_pinned(
    tool: Tool,
    pin: &Pinned,
    archive: &Archive,
    from: &Path,
    staging: &Path,
    dest: &Path,
) -> Result<PathBuf, ToolError> {
    let target = dest.parent().expect("the program is in its folder");
    let version = target.parent().expect("its folder is in its version's");
    zajil_platform::fs::private_dir(version)?;
    let mut source = zajil_platform::fs::open_regular_nofollow(from, false)?;
    let actual = {
        let mut hasher = Sha256::new();
        io::copy(&mut source, &mut HashWriter(&mut hasher))?;
        hex(&hasher.finalize())
    };
    if actual != pin.sha256 {
        return Err(ToolError::Mismatch {
            tool,
            actual,
            expected: pin.sha256.clone(),
        });
    }
    let new = staging.join("tool.new");
    // The same file, read again from its start, now as a zip.
    std::io::Seek::rewind(&mut source)?;
    let tree = unpack_zip(source, &new, &archive.program)
        .map_err(|why| ToolError::BadArchive(tool, why))?;
    if tree != archive.tree_sha256 {
        return Err(ToolError::Mismatch {
            tool,
            actual: tree,
            expected: archive.tree_sha256.clone(),
        });
    }
    unmark_all(&new)?;
    match std::fs::symlink_metadata(target) {
        Ok(_) => zajil_platform::fs::remove_dir_all_retrying(target)?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    // Again for a while if a scanner still holds a file just written (#55;
    // review of #278); `target` is gone, so nothing is replaced.
    zajil_platform::fs::rename_noreplace(&new, target)?;
    zajil_platform::fs::sync_dir(version)?;
    Ok(dest.to_owned())
}

/// Every file under `dir` (a tool unpacked from a zip), checked as each
/// tool file is right before it runs: each folder one only this user can
/// change, each file a regular one only this user can change, opened and
/// hashed; nothing else (no link). Returns the [`tree_sha256`] and the
/// opened files by their path inside `dir`.
fn checked_tree(tool: Tool, dir: &Path) -> Result<(String, Vec<(String, File)>), ToolError> {
    let mut sums: Vec<(String, String)> = Vec::new();
    let mut files: Vec<(String, File)> = Vec::new();
    let mut folders: Vec<(PathBuf, String)> = vec![(dir.to_path_buf(), String::new())];
    while let Some((folder, inside)) = folders.pop() {
        if !zajil_platform::fs::is_private_dir(&folder)? {
            return Err(ToolError::NotPrivate(folder));
        }
        for entry in std::fs::read_dir(&folder)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ToolError::BadArchive(tool, "a name that is not UTF-8".into()))?;
            let path = entry.path();
            let relative = if inside.is_empty() {
                name
            } else {
                format!("{inside}/{name}")
            };
            if sums.len() + folders.len() > MAX_ENTRIES {
                return Err(ToolError::BadArchive(tool, "too many files".into()));
            }
            let meta = std::fs::symlink_metadata(&path)?;
            if meta.is_dir() {
                folders.push((path, relative));
                continue;
            }
            if !meta.is_file() {
                return Err(ToolError::NotPrivate(path));
            }
            let mut file = match zajil_platform::fs::open_private_tool(&path) {
                Ok(file) => file,
                Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
                    return Err(ToolError::NotPrivate(path));
                }
                Err(err) => return Err(err.into()),
            };
            let mut hasher = Sha256::new();
            io::copy(&mut file, &mut HashWriter(&mut hasher))?;
            sums.push((relative.clone(), hex(&hasher.finalize())));
            files.push((relative, file));
        }
    }
    Ok((tree_sha256(&sums), files))
}

/// After `tool` was installed: what an earlier version or build of it left
/// in its folder goes (yt-dlp's one-file build, when its zip came; a
/// version no longer pinned). Best effort: a file in use (a run of the
/// earlier one, on Windows) stays until the next install.
fn remove_earlier(root: &Path, tool: Tool, pin: &Pinned) {
    let folder = root.join(tool.folder());
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name() == std::ffi::OsStr::new(&pin.version) {
            continue;
        }
        let _ = remove_entry(&entry.path());
    }
    // In the version's own folder, what is not this tool's: only for a zip,
    // whose folder holds nothing else (ffmpeg and ffprobe share theirs).
    if let Some(archive) = &pin.archive {
        let Ok(entries) = std::fs::read_dir(folder.join(&pin.version)) else {
            return;
        };
        for entry in entries.flatten() {
            if entry.file_name() != std::ffi::OsStr::new(&archive.folder) {
                let _ = remove_entry(&entry.path());
            }
        }
    }
}

/// Removes a file or a folder with what it holds (a link itself, never what
/// it points to).
fn remove_entry(path: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(path)?.is_dir() {
        zajil_platform::fs::remove_dir_all_retrying(path)
    } else {
        zajil_platform::fs::remove_file_retrying(path)
    }
}

fn copy_hashing_from(from: &mut impl Read, to: &mut File) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = from.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        to.write_all(&buf[..n])?;
    }
    Ok(hex(&hasher.finalize()))
}

fn copy_hashing(from: &mut File, to: &mut File) -> io::Result<String> {
    copy_hashing_from(from, to)
}

/// A tool checked right before it runs: its path, and the files it was
/// checked on, kept open until its run has ended: on Windows each stays
/// locked while open. For a zip, everything unpacked from it is kept open
/// on Windows only: elsewhere an open file locks nothing, and a Mac app may
/// have only 256 files open at once (review of #278).
#[derive(Debug)]
pub struct VerifiedTool {
    path: PathBuf,
    _files: Vec<File>,
}

impl VerifiedTool {
    /// The program to run.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn check_installed(
    tool: Tool,
    pin: &Pinned,
    root: &Path,
    path: &Path,
) -> Result<VerifiedTool, ToolError> {
    // Every folder from the tools folder down to the file: no other user
    // may change, rename or replace any of them (review of the video
    // tools: only the last folder was checked).
    let dir = path.parent().expect("a tool path has a folder");
    let below = dir.strip_prefix(root).map_err(io::Error::other)?;
    let mut folder = root.to_path_buf();
    for level in std::iter::once(None).chain(below.components().map(Some)) {
        if let Some(part) = level {
            folder.push(part);
        }
        match zajil_platform::fs::is_private_dir(&folder) {
            Ok(true) => {}
            Ok(false) => return Err(ToolError::NotPrivate(folder)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Err(ToolError::Missing(tool));
            }
            Err(err) => return Err(err.into()),
        }
    }
    if let Some(archive) = &pin.archive {
        let (tree, files) = checked_tree(tool, dir)?;
        if tree != archive.tree_sha256 {
            return Err(ToolError::Mismatch {
                tool,
                actual: tree,
                expected: archive.tree_sha256.clone(),
            });
        }
        if !files.iter().any(|(name, _)| name == &archive.program) {
            return Err(ToolError::Missing(tool));
        }
        unmark_all(dir)?;
        unmark_folders(root, dir)?;
        let kept = if cfg!(windows) {
            files.into_iter().map(|(_, file)| file).collect()
        } else {
            Vec::new()
        };
        return Ok(VerifiedTool {
            path: path.to_owned(),
            _files: kept,
        });
    }
    // The file itself, checked on the opened file.
    let mut file = match zajil_platform::fs::open_private_tool(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Err(ToolError::Missing(tool)),
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
            return Err(ToolError::NotPrivate(path.to_owned()));
        }
        Err(err) => return Err(err.into()),
    };
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut HashWriter(&mut hasher))?;
    let actual = hex(&hasher.finalize());
    if actual != pin.sha256 {
        return Err(ToolError::Mismatch {
            tool,
            actual,
            expected: pin.sha256.clone(),
        });
    }
    unmark(path)?;
    unmark_folders(root, dir)?;
    Ok(VerifiedTool {
        path: path.to_owned(),
        _files: vec![file],
    })
}

/// Removes the mark of the internet from `path` itself, on macOS: a Zajil
/// started by a marked process (Chrome, through the extension, before the
/// host started it through LaunchServices) is marked too, and the system
/// then marks every file it writes and every file its programs write;
/// Gatekeeper refused yt-dlp's Python.framework so marked as "damaged"
/// (owner's report before 1.0). Only on what was just checked to be the
/// pinned tool, and the folders that hold it. Windows and Linux mark
/// nothing by themselves. Blocking.
fn unmark(path: &Path) -> io::Result<()> {
    if cfg!(target_os = "macos") {
        zajil_platform::origin::clear(path)
    } else {
        Ok(())
    }
}

/// [`unmark`] on `path` and, for a folder, everything in it (a link itself,
/// never what it points to).
fn unmark_all(path: &Path) -> io::Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    unmark(path)?;
    if std::fs::symlink_metadata(path)?.is_dir() {
        for entry in std::fs::read_dir(path)? {
            unmark_all(&entry?.path())?;
        }
    }
    Ok(())
}

/// [`unmark`] on each folder from `root` down to `dir`, each itself only.
fn unmark_folders(root: &Path, dir: &Path) -> io::Result<()> {
    let below = dir.strip_prefix(root).map_err(io::Error::other)?;
    let mut folder = root.to_path_buf();
    unmark(&folder)?;
    for part in below.components() {
        folder.push(part);
        unmark(&folder)?;
    }
    Ok(())
}

struct HashWriter<'a>(&'a mut Sha256);

impl Write for HashWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_target_has_a_pinned_yt_dlp_and_quickjs_over_https() {
        for (tool, release) in [
            (
                Tool::YtDlp,
                "https://github.com/yt-dlp/yt-dlp/releases/download/",
            ),
            (
                Tool::QuickJs,
                "https://github.com/quickjs-ng/quickjs/releases/download/v",
            ),
        ] {
            for os in [Os::Windows, Os::Macos, Os::Linux] {
                for arch in [Arch::X86_64, Arch::Aarch64] {
                    let pin = pinned(tool, Target { os, arch }).expect("a pin for every target");
                    assert!(pin.url.starts_with(release), "{}", pin.url);
                    assert_eq!(pin.sha256.len(), 64);
                    assert!(
                        pin.sha256
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                    );
                }
            }
        }
        assert_eq!(
            pinned(
                Tool::YtDlp,
                Target {
                    os: Os::Linux,
                    arch: Arch::X86_64
                }
            )
            .unwrap()
            .version,
            yt_dlp_version()
        );
        // No Windows arm64 build of QuickJS-NG: the x86_64 one, emulated.
        let win = |arch| {
            pinned(
                Tool::QuickJs,
                Target {
                    os: Os::Windows,
                    arch,
                },
            )
            .unwrap()
        };
        assert_eq!(win(Arch::Aarch64), win(Arch::X86_64));
    }

    #[test]
    fn this_build_s_pins_files_are_valid() {
        for text in [YT_DLP_PINS, QUICKJS_PINS, FFMPEG_PINS, FFPROBE_PINS] {
            let pins = parse_pins(text).unwrap();
            assert_eq!(pins.files.len(), KNOWN_TARGETS.len());
        }
    }

    /// Zajil's FFmpeg build: every target, from this repository's
    /// `tools-ffmpeg-<version>` release; ffmpeg and ffprobe of one version
    /// (they share a folder); both Macs the universal file; each file named
    /// as the release names it. The version is FFmpeg's and a build number,
    /// never newer than source.env: a new source.env comes first, then its
    /// release (tagged on main), then its pins (review of #164).
    #[test]
    fn every_target_has_our_ffmpeg_and_ffprobe_of_one_version() {
        let source = include_str!("../../../tools/ffmpeg/source.env")
            .lines()
            .find_map(|l| l.strip_prefix("FFMPEG_VERSION="))
            .unwrap();
        let version = &ffmpeg_pins().version;
        assert_eq!(&ffprobe_pins().version, version);
        let numbers = |v: &str| -> Vec<u32> { v.split('.').map(|n| n.parse().unwrap()).collect() };
        let (ffmpeg, build) = version.rsplit_once('-').expect("<ffmpeg version>-<build>");
        assert!(build.parse::<u32>().is_ok(), "{version}");
        assert_eq!(numbers(ffmpeg).len(), 3, "{version}");
        assert!(
            numbers(ffmpeg) <= numbers(source),
            "{version} is newer than {source}"
        );
        for tool in [Tool::Ffmpeg, Tool::Ffprobe] {
            for os in [Os::Windows, Os::Macos, Os::Linux] {
                for arch in [Arch::X86_64, Arch::Aarch64] {
                    let target = Target { os, arch };
                    let pin = pinned(tool, target).expect("a pin for every target");
                    assert_eq!(&pin.version, version);
                    let file = match os {
                        Os::Windows => format!("{tool}-{}.exe", target_name(target)),
                        Os::Macos => format!("{tool}-macos-universal"),
                        Os::Linux => format!("{tool}-{}", target_name(target)),
                    };
                    assert_eq!(
                        pin.url,
                        format!(
                            "https://github.com/motalim-ai/zajil-download-manager/releases/download/tools-ffmpeg-{version}/{file}"
                        )
                    );
                }
            }
        }
        // The FFmpeg notice links the source of the build Zajil downloads
        // (ADR-0009, LGPL checklist, item 9).
        let release = format!("releases/tag/tools-ffmpeg-{version})");
        assert!(
            include_str!("../../../README.md").contains(&release),
            "README.md: {release}"
        );
        // And the notices file in every installer (item 11), in both
        // languages.
        let notices = include_str!("../../../scripts/release/licenses/THIRD-PARTY-NOTICES.md");
        assert_eq!(notices.matches(&release).count(), 2, "{release}");
        assert_eq!(
            notices.matches(&format!("`{version}`")).count(),
            2,
            "{version}"
        );
        let mac = |tool, arch| {
            pinned(
                tool,
                Target {
                    os: Os::Macos,
                    arch,
                },
            )
            .unwrap()
        };
        assert_eq!(
            mac(Tool::Ffmpeg, Arch::X86_64),
            mac(Tool::Ffmpeg, Arch::Aarch64)
        );
        assert_ne!(
            mac(Tool::Ffmpeg, Arch::X86_64),
            mac(Tool::Ffprobe, Arch::X86_64)
        );
    }

    #[test]
    fn a_pins_file_is_read_strictly() {
        let sha = "a".repeat(64);
        let good = format!("# yt-dlp\nversion 2026.10.21\n\nlinux-x86_64 yt-dlp_linux {sha}\n");
        assert_eq!(parse_pins(&good).unwrap().files.len(), 1);
        let zip = format!("version 1\nmacos-aarch64 yt-dlp_macos.zip {sha} yt-dlp_macos {sha}\n");
        assert_eq!(
            parse_pins(&zip).unwrap().files[0].archive,
            Some(("yt-dlp_macos".to_owned(), sha.clone()))
        );
        for bad in [
            // A zip needs its program and tree; nothing else may have them.
            format!("version 1\nmacos-aarch64 yt-dlp_macos.zip {sha}"),
            format!("version 1\nmacos-aarch64 yt-dlp_macos {sha} yt-dlp_macos {sha}"),
            format!("version 1\nmacos-aarch64 f.zip {sha} ../p {sha}"),
            format!("version 1\nmacos-aarch64 f.zip {sha} p {}", "A".repeat(64)),
            format!("version 1\nmacos-aarch64 f.zip {sha} p"),
            format!("version 1\nmacos-aarch64 .zip {sha} p {sha}"),
        ] {
            assert!(parse_pins(&bad).is_err(), "{bad:?}");
        }
        for bad in [
            String::new(),
            format!("linux-x86_64 yt-dlp_linux {sha}"),
            format!("version 1\nversion 2\nlinux-x86_64 f {sha}"),
            format!("version ../x\nlinux-x86_64 f {sha}"),
            format!("version ..\nlinux-x86_64 f {sha}"),
            format!("version 1\nlinux-x86_64 f {sha} extra"),
            format!("version 1\nlinux-x86_64 f {sha}\nlinux-x86_64 g {sha}"),
            format!("version 1\nbeos-x86 f {sha}"),
            format!("version 1\nlinux-x86_64 ../f {sha}"),
            format!("version 1\nlinux-x86_64 f {}", "A".repeat(64)),
            format!("version 1\nlinux-x86_64 f {}", "a".repeat(63)),
            "version 1\nsomething".to_owned(),
        ] {
            assert!(parse_pins(&bad).is_err(), "{bad:?}");
        }
    }

    /// yt-dlp on macOS and Windows is the official zip (owner's decision,
    /// 2026-10-27), unpacked into a folder named after it; Linux keeps the
    /// one-file build.
    #[test]
    fn yt_dlp_is_a_zip_on_macos_and_windows_only() {
        for os in [Os::Windows, Os::Macos, Os::Linux] {
            for arch in [Arch::X86_64, Arch::Aarch64] {
                let pin = pinned(Tool::YtDlp, Target { os, arch }).unwrap();
                match (&pin.archive, os) {
                    (Some(a), Os::Windows | Os::Macos) => {
                        assert!(
                            pin.url.ends_with(&format!("/{}.zip", a.folder)),
                            "{}",
                            pin.url
                        );
                        assert!(a.program.starts_with("yt-dlp"));
                        assert_eq!(a.program.ends_with(".exe"), os == Os::Windows);
                    }
                    (None, Os::Linux) => {}
                    _ => panic!("{os:?} {arch:?}: {pin:?}"),
                }
            }
        }
        let root = Path::new("/tools");
        let toolbox = Toolbox {
            root: root.to_path_buf(),
            target: Some(Target {
                os: Os::Windows,
                arch: Arch::Aarch64,
            }),
            pins: Box::new(pinned),
        };
        assert_eq!(
            toolbox.path(Tool::YtDlp).unwrap(),
            root.join("yt-dlp")
                .join(yt_dlp_version())
                .join("yt-dlp_win_arm64")
                .join("yt-dlp_arm64.exe")
        );
    }

    #[test]
    fn tools_keep_their_own_names() {
        let windows = Target {
            os: Os::Windows,
            arch: Arch::X86_64,
        };
        let linux = Target {
            os: Os::Linux,
            arch: Arch::Aarch64,
        };
        assert_eq!(Tool::Ffmpeg.file_name(windows), "ffmpeg.exe");
        assert_eq!(Tool::Ffprobe.file_name(linux), "ffprobe");
        assert_eq!(Tool::YtDlp.file_name(linux), "yt-dlp");
        assert_eq!(Tool::Ffmpeg.folder(), Tool::Ffprobe.folder());
    }
}
