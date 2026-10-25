//! The programs Zajil runs for video downloads (ADR-0009): yt-dlp, and our
//! own LGPL build of ffmpeg and ffprobe.
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
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::YtDlp, Tool::Ffmpeg, Tool::Ffprobe];

    /// The name the program is saved under (with `.exe` on Windows). FFmpeg's
    /// files keep FFmpeg's own names (LGPL checklist, item 16).
    pub fn file_name(self, target: Target) -> String {
        let base = match self {
            Tool::YtDlp => "yt-dlp",
            Tool::Ffmpeg => "ffmpeg",
            Tool::Ffprobe => "ffprobe",
        };
        format!("{base}{}", target.exe_suffix())
    }

    /// The folder of its version under the tools folder: ffmpeg and ffprobe
    /// share one, so yt-dlp finds both through `--ffmpeg-location`.
    fn folder(self) -> &'static str {
        match self {
            Tool::YtDlp => "yt-dlp",
            Tool::Ffmpeg | Tool::Ffprobe => "ffmpeg",
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
/// `<os>-<arch> <release file> <sha256>` lines. One file for the program
/// and the CI, which downloads the same files for its tests. A newer
/// version comes with a new Zajil release, after its tests pass (ADR-0009,
/// section 7).
const YT_DLP_PINS: &str = include_str!("../../../tools/yt-dlp/pins.txt");

/// The pinned version of yt-dlp.
pub fn yt_dlp_version() -> &'static str {
    YT_DLP_PINS
        .lines()
        .find_map(|line| line.strip_prefix("version "))
        .expect("pins.txt has a version line")
        .trim()
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

/// The pinned yt-dlp file for `target`, if pins.txt has one.
fn yt_dlp(target: Target) -> Option<Pinned> {
    let wanted = target_name(target);
    let version = yt_dlp_version();
    YT_DLP_PINS.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next()? == wanted).then_some(())?;
        let (file, sha256) = (words.next()?, words.next()?);
        Some(Pinned {
            version: version.to_owned(),
            url: format!("https://github.com/yt-dlp/yt-dlp/releases/download/{version}/{file}"),
            sha256: sha256.to_owned(),
        })
    })
}

/// What a [`Toolbox`] downloads: a tool and a target to the pinned file,
/// or `None` while that file is not released yet.
pub type Pins = dyn Fn(Tool, Target) -> Option<Pinned> + Send + Sync;

/// The files pinned in this build of Zajil.
pub fn pinned(tool: Tool, target: Target) -> Option<Pinned> {
    match tool {
        Tool::YtDlp => yt_dlp(target),
        // Pinned once the first tools-ffmpeg release is published by
        // .github/workflows/ffmpeg.yml.
        Tool::Ffmpeg | Tool::Ffprobe => None,
    }
}

/// Why a tool cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("{0:?} has no released file for this system yet")]
    NotReleased(Tool),
    #[error("this system or processor has no video tools")]
    UnknownTarget,
    #[error("{0:?} is not downloaded yet")]
    Missing(Tool),
    /// The file is not the pinned one: refused, and removed if it was a
    /// new download.
    #[error("{tool:?} is not the expected file (SHA-256 {actual}, expected {expected})")]
    Mismatch {
        tool: Tool,
        actual: String,
        expected: String,
    },
    #[error("{} can be changed by another user, so it is not run", .0.display())]
    NotPrivate(PathBuf),
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
        Ok((pin, target))
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

    /// The path of `tool`, checked right before it runs: the pinned file,
    /// which no other user can change, in a folder no other user can
    /// change.
    pub async fn verified(&self, tool: Tool) -> Result<PathBuf, ToolError> {
        let (pin, path) = (self.pin(tool)?.0, self.path(tool)?);
        tokio::task::spawn_blocking(move || check_installed(tool, &pin, &path))
            .await
            .map_err(io::Error::other)?
    }

    fn verified_blocking(&self, tool: Tool) -> Result<PathBuf, ToolError> {
        let (pin, path) = (self.pin(tool)?.0, self.path(tool)?);
        check_installed(tool, &pin, &path)
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
            let result = keep_if_pinned(tool, &pin, &finished.file, &dest);
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
/// internet (ADR-0009, section 2) and is not the engine's file.
fn keep_if_pinned(
    tool: Tool,
    pin: &Pinned,
    from: &Path,
    dest: &Path,
) -> Result<PathBuf, ToolError> {
    let dir = dest.parent().expect("a tool path has a folder");
    zajil_platform::fs::private_dir(dir)?;
    let mut source = zajil_platform::fs::open_regular_nofollow(from, false)?;
    let new = dest.with_extension("new");
    match std::fs::remove_file(&new) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err.into()),
        _ => {}
    }
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

fn check_installed(tool: Tool, pin: &Pinned, path: &Path) -> Result<PathBuf, ToolError> {
    let mut file = match zajil_platform::fs::open_regular_nofollow(path, false) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Err(ToolError::Missing(tool)),
        Err(err) => return Err(err.into()),
    };
    let dir = path.parent().expect("a tool path has a folder");
    for checked in [path, dir] {
        if !zajil_platform::fs::only_user_or_root_can_change(checked)? {
            return Err(ToolError::NotPrivate(checked.to_owned()));
        }
    }
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
    Ok(path.to_owned())
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
    fn every_target_has_a_pinned_yt_dlp_over_https() {
        for os in [Os::Windows, Os::Macos, Os::Linux] {
            for arch in [Arch::X86_64, Arch::Aarch64] {
                let pin = yt_dlp(Target { os, arch }).expect("a pin for every target");
                assert!(
                    pin.url
                        .starts_with("https://github.com/yt-dlp/yt-dlp/releases/download/")
                );
                assert_eq!(pin.sha256.len(), 64);
                assert!(
                    pin.sha256
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                );
                assert_eq!(pin.version, yt_dlp_version());
            }
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
