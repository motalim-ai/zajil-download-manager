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
    /// `(<os>-<arch>, release file, sha256)`, each target once.
    pub files: Vec<(String, String, String)>,
}

/// Reads a pins file strictly (review of the video tools): comments (`#`)
/// and empty lines, one `version <version>` line, then exactly three
/// fields per line: a known target, once; a plain file name; a lower-case
/// SHA-256. The version and file name are plain names (letters, digits,
/// `-`, `_`, `.`, never `.` or `..`): the version names a folder.
pub fn parse_pins(text: &str) -> Result<PinsFile, String> {
    let mut version = None;
    let mut files: Vec<(String, String, String)> = Vec::new();
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
            [target, file, sha256] => {
                if !KNOWN_TARGETS.contains(target) {
                    return Err(format!("{}: unknown target {target:?}", at()));
                }
                if files.iter().any(|(t, _, _)| t == target) {
                    return Err(format!("{}: {target} twice", at()));
                }
                if !is_plain_name(file) {
                    return Err(format!("{}: the file is not a plain name", at()));
                }
                let hex = sha256.len() == 64
                    && sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                if !hex {
                    return Err(format!("{}: not a lower-case SHA-256", at()));
                }
                files.push((
                    (*target).to_owned(),
                    (*file).to_owned(),
                    (*sha256).to_owned(),
                ));
            }
            _ => {
                return Err(format!(
                    "{}: not `version <v>` nor `<target> <file> <sha256>`",
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
    pins.files
        .iter()
        .find(|(t, _, _)| *t == wanted)
        .map(|(_, file, sha256)| Pinned {
            version: version.clone(),
            url: format!(
                "https://github.com/{repository}/releases/download/{tag_prefix}{version}/{file}"
            ),
            sha256: sha256.clone(),
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
        if !is_plain_name(&pin.version) || !pin.url.starts_with("https://") {
            return Err(ToolError::InvalidPin(tool));
        }
        Ok((pin, target))
    }

    /// The pinned SHA-256 of `tool` for this system (lower-case hex).
    pub fn sha256(&self, tool: Tool) -> Result<String, ToolError> {
        Ok(self.pin(tool)?.0.sha256)
    }

    /// Where `tool` is, or would be, kept.
    pub fn path(&self, tool: Tool) -> Result<PathBuf, ToolError> {
        let (pin, target) = self.pin(tool)?;
        Ok(self
            .root
            .join(tool.folder())
            .join(&pin.version)
            .join(tool.file_name(target)))
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
        tokio::task::spawn_blocking(move || {
            let result = keep_if_pinned(tool, &pin, &finished.file, staging.path(), &dest);
            staging.remove();
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

    /// Removes the folder now. Blocking.
    fn remove(mut self) {
        if let Some(dir) = self.0.take() {
            let _ = std::fs::remove_dir_all(dir);
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
/// internet (ADR-0009, section 2) and is not the engine's file. The copy is
/// made in `staging`, this install's own folder on the same volume, so two
/// installs at once never share a file (review of the video tools).
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
    zajil_platform::fs::make_executable(&out)?;
    drop(out);
    zajil_platform::fs::replace_file(&new, dest)?;
    zajil_platform::fs::sync_dir(dir)?;
    Ok(dest.to_owned())
}

fn copy_hashing(from: &mut File, to: &mut File) -> io::Result<String> {
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

/// A tool checked right before it runs: its path, and the file it was
/// checked on, kept open until the program has started (on Windows it
/// stays locked while open).
#[derive(Debug)]
pub struct VerifiedTool {
    path: PathBuf,
    _file: File,
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
    Ok(VerifiedTool {
        path: path.to_owned(),
        _file: file,
    })
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
