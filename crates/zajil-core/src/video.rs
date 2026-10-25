//! Video downloads with yt-dlp and our ffmpeg (ADR-0009, sections 4 to 6).
//!
//! Every run of yt-dlp:
//! - starts the checked tool directly, never through a shell, as a process
//!   tree that ends with it and with Zajil ([`zajil_platform::process`]);
//! - gets only arguments built here from a fixed set ([`Arg`]): the page's
//!   URL is checked and comes last, alone, after `--`; nothing from the page
//!   ever takes the place of an option, a path or an output template;
//! - ignores the user's yt-dlp configuration and plugins, fetches no code,
//!   uses yt-dlp's own downloader, and finds only our ffmpeg;
//! - has a private folder of its own under `<data dir>/run` (its temporary
//!   files, the cookie file when there is one, and a record of its process
//!   tree), removed as soon as the tree has ended, and swept at the next
//!   start after a crash ([`Runs::clean_leftovers`]);
//! - writes the video into a private folder next to where it goes, under a
//!   fixed name; Zajil names the final file itself.
//!
//! Content protected by DRM is never downloaded (AGENTS.md, section 6).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

use crate::engine::{Engine, Stop};

/// The tools a run needs, as [`crate::tools::Toolbox::verified`] gave them
/// (tests give their own).
#[derive(Debug, Clone)]
pub struct VideoTools {
    /// The yt-dlp program.
    pub yt_dlp: PathBuf,
    /// The folder holding our ffmpeg and ffprobe.
    pub ffmpeg_dir: PathBuf,
}

/// The longest JSON a probe may print (a long playlist, or a page with
/// many formats), and the longest line of anything else.
const MAX_JSON: u64 = 64 * 1024 * 1024;
const MAX_LINE: usize = 64 * 1024;
/// The most entries of a playlist that are listed.
pub const MAX_PLAYLIST_ENTRIES: usize = 1000;

/// One argument yt-dlp may get. Every argument of every run is one of
/// these: there is no way to pass anything else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    IgnoreConfig,
    NoPluginDirs,
    NoRemoteComponents,
    NoCacheDir,
    NoJsRuntimes,
    NoPlaylist,
    FlatPlaylist,
    NoColor,
    NoMtime,
    Progress,
    Newline,
    DumpSingleJson,
    /// Utf-8 output, whatever the system's console uses.
    EncodingUtf8,
    /// yt-dlp's own downloader for every protocol (HLS included), never
    /// one that hands a URL to ffmpeg.
    DownloaderNative,
    /// The folder of our ffmpeg.
    FfmpegLocation(PathBuf),
    /// The cookie file of this run.
    Cookies(PathBuf),
    /// The fixed, relative output name `media.%(ext)s`.
    Output,
    /// A format checked by [`check_format_id`] (or two joined by `+`).
    Format(String),
    /// Audio only, as m4a (AAC).
    AudioOnlyM4a,
    /// Prints the final path, once moved, for Zajil to read.
    PrintFinalPath,
    /// Prints progress in a fixed form Zajil reads.
    ProgressTemplate,
}

/// The progress line Zajil asks for: a fixed word, then numbers or `NA`.
const PROGRESS_PREFIX: &str = "zajil-progress";

impl Arg {
    fn push(&self, out: &mut Vec<std::ffi::OsString>) {
        let mut add = |s: &str| out.push(s.into());
        match self {
            Arg::IgnoreConfig => add("--ignore-config"),
            Arg::NoPluginDirs => add("--no-plugin-dirs"),
            Arg::NoRemoteComponents => add("--no-remote-components"),
            Arg::NoCacheDir => add("--no-cache-dir"),
            Arg::NoJsRuntimes => add("--no-js-runtimes"),
            Arg::NoPlaylist => add("--no-playlist"),
            Arg::FlatPlaylist => add("--flat-playlist"),
            Arg::NoColor => add("--no-color"),
            Arg::NoMtime => add("--no-mtime"),
            Arg::Progress => add("--progress"),
            Arg::Newline => add("--newline"),
            Arg::DumpSingleJson => add("--dump-single-json"),
            Arg::EncodingUtf8 => {
                add("--encoding");
                add("utf-8");
            }
            Arg::DownloaderNative => {
                add("--downloader");
                add("native");
            }
            Arg::FfmpegLocation(dir) => {
                add("--ffmpeg-location");
                out.push(dir.as_os_str().to_owned());
            }
            Arg::Cookies(file) => {
                add("--cookies");
                out.push(file.as_os_str().to_owned());
            }
            Arg::Output => {
                add("--output");
                add("media.%(ext)s");
            }
            Arg::Format(format) => {
                add("--format");
                add(format);
            }
            Arg::AudioOnlyM4a => {
                add("--extract-audio");
                add("--audio-format");
                add("m4a");
            }
            Arg::PrintFinalPath => {
                add("--print");
                add("after_move:filepath");
            }
            Arg::ProgressTemplate => {
                add("--progress-template");
                add(&format!(
                    "download:{PROGRESS_PREFIX} %(progress.downloaded_bytes)s %(progress.total_bytes)s \
                     %(progress.total_bytes_estimate)s %(progress.speed)s"
                ));
            }
        }
    }
}

/// The arguments for `args`, then `--` and the URL, alone, last.
pub fn command_line(args: &[Arg], url: &str) -> Vec<std::ffi::OsString> {
    let mut out = Vec::new();
    for arg in args {
        arg.push(&mut out);
    }
    out.push("--".into());
    out.push(url.into());
    out
}

/// What every run gets, before what the run itself needs.
fn base_args(tools: &VideoTools) -> Vec<Arg> {
    vec![
        Arg::IgnoreConfig,
        Arg::NoPluginDirs,
        Arg::NoRemoteComponents,
        Arg::NoCacheDir,
        // No JavaScript runtime until YouTube's own PR (ADR-0009, section 3).
        Arg::NoJsRuntimes,
        Arg::NoColor,
        Arg::EncodingUtf8,
        Arg::DownloaderNative,
        Arg::FfmpegLocation(tools.ffmpeg_dir.clone()),
    ]
}

/// Checks a format id from a probe before it becomes `--format`: letters,
/// digits, `_`, `.` and `-` only, since `/`, `+`, `[`, `,` and others are
/// operators of yt-dlp's format language.
pub fn check_format_id(id: &str) -> Result<(), VideoError> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
        && !id.starts_with('-');
    if ok {
        Ok(())
    } else {
        Err(VideoError::Refused(format!("not a format id: {id:?}")))
    }
}

/// What to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// One format with both picture and sound, or audio alone.
    Format(String),
    /// A picture and a sound format, merged by ffmpeg without re-encoding.
    Merge { video: String, audio: String },
    /// Sound only, as m4a: the best m4a the site offers, else the best
    /// sound turned into AAC by our ffmpeg.
    AudioOnly,
}

impl Choice {
    fn args(&self) -> Result<Vec<Arg>, VideoError> {
        Ok(match self {
            Choice::Format(id) => {
                check_format_id(id)?;
                vec![Arg::Format(id.clone())]
            }
            Choice::Merge { video, audio } => {
                check_format_id(video)?;
                check_format_id(audio)?;
                vec![Arg::Format(format!("{video}+{audio}"))]
            }
            Choice::AudioOnly => vec![
                Arg::Format("bestaudio[ext=m4a]/bestaudio".to_owned()),
                Arg::AudioOnlyM4a,
            ],
        })
    }
}

/// Why a video cannot be downloaded.
#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    #[error("{0}")]
    Refused(String),
    /// Protected by DRM: never downloaded.
    #[error("this video is protected (DRM), and Zajil does not download protected content")]
    Drm,
    /// What yt-dlp said, without the secrets a URL may carry.
    #[error("yt-dlp: {0}")]
    Tool(String),
    #[error("yt-dlp's output was not what Zajil expects: {0}")]
    Output(String),
    #[error("stopped")]
    Stopped,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// A format a page offers, as the probe found it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Format {
    #[serde(rename = "format_id")]
    pub id: String,
    #[serde(default)]
    pub ext: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub fps: Option<f64>,
    /// `none` for a format without picture.
    #[serde(default)]
    pub vcodec: Option<String>,
    /// `none` for a format without sound.
    #[serde(default)]
    pub acodec: Option<String>,
    #[serde(default)]
    pub filesize: Option<u64>,
    #[serde(default)]
    pub filesize_approx: Option<u64>,
    /// Total bitrate in kbit/s.
    #[serde(default)]
    pub tbr: Option<f64>,
    #[serde(default)]
    pub has_drm: Option<serde_json::Value>,
    /// How yt-dlp gets it (`https`, `m3u8_native`, `http_dash_segments`…).
    #[serde(default)]
    pub protocol: Option<String>,
    /// Its URL (for an HLS format, its playlist), kept in memory only for
    /// Zajil's own DRM check: it may carry a token, so it is never shown.
    #[serde(default)]
    url: Option<Hidden>,
}

/// A value that may hold a secret: printed as `(hidden)`.
#[derive(Clone, PartialEq, Deserialize)]
#[serde(transparent)]
struct Hidden(String);

impl std::fmt::Debug for Hidden {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(hidden)")
    }
}

impl Format {
    pub fn has_video(&self) -> bool {
        self.vcodec.as_deref().is_some_and(|c| c != "none")
    }

    pub fn has_audio(&self) -> bool {
        self.acodec.as_deref().is_some_and(|c| c != "none")
    }

    /// Whether yt-dlp gets it through an HLS playlist.
    pub fn is_hls(&self) -> bool {
        self.protocol
            .as_deref()
            .is_some_and(|p| p.starts_with("m3u8"))
    }

    /// yt-dlp marks protected formats with `has_drm` true (or `"maybe"`).
    pub fn is_protected(&self) -> bool {
        match &self.has_drm {
            Some(serde_json::Value::Bool(b)) => *b,
            Some(serde_json::Value::String(s)) => !s.is_empty(),
            _ => false,
        }
    }
}

/// A single video.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Video {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub formats: Vec<Format>,
}

/// One item of a playlist, to be probed and downloaded on its own.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

/// What a page holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Probed {
    Video(Video),
    Playlist { title: String, entries: Vec<Entry> },
}

/// Only the fields Zajil uses are read from the probe's JSON: the rest
/// (each format's `http_headers` and `cookies` among them) is never kept,
/// stored or printed (AGENTS.md, section 10.3).
#[derive(Deserialize)]
struct Raw {
    #[serde(rename = "_type", default)]
    kind: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    formats: Vec<Format>,
    #[serde(default)]
    entries: Vec<Entry>,
}

/// What a download reports while it runs.
#[derive(Debug, Clone, PartialEq)]
pub enum VideoEvent {
    /// Bytes of the current part (the picture, then the sound) so far.
    Progress {
        done: u64,
        total: Option<u64>,
        speed: Option<u64>,
    },
}

/// The folders of runs, `<data dir>/run`.
pub struct Runs {
    root: PathBuf,
}

const RECORD_FILE: &str = "tree";
const OUT_FILE: &str = "out";

impl Runs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Runs { root: root.into() }
    }

    /// At program start, before any run: ends every process tree a crash
    /// left running, then removes the folders of those runs and their
    /// output folders. Blocking.
    pub fn clean_leftovers(&self) -> io::Result<()> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err),
        };
        for entry in entries {
            let dir = entry?.path();
            if let Ok(line) = read_small(&dir.join(RECORD_FILE))
                && let Some(record) = zajil_platform::process::Record::from_line(&line)
            {
                zajil_platform::process::end_leftover(record)?;
            }
            if let Ok(out) = read_small(&dir.join(OUT_FILE)) {
                let out = PathBuf::from(out.trim_end_matches('\n'));
                // Only an output folder of ours: a private `.zajil-video-*`.
                if out
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(".zajil-video-"))
                {
                    let _ = std::fs::remove_dir_all(&out);
                }
            }
            std::fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }
}

/// A small text file, never through a link, at most 64 KiB.
fn read_small(path: &Path) -> io::Result<String> {
    let file = zajil_platform::fs::open_regular_nofollow(path, false)?;
    let mut text = String::new();
    file.take(64 * 1024).read_to_string(&mut text)?;
    Ok(text)
}

fn random_hex() -> io::Result<String> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// One run's folders: its private folder under `<data dir>/run`, and, for
/// a download, its private output folder next to the destination. Both
/// are removed when it is dropped; the tree has ended by then.
struct Run {
    dir: PathBuf,
    out: Option<PathBuf>,
}

impl Run {
    fn start(runs: &Runs, dest: Option<&Path>) -> io::Result<Run> {
        zajil_platform::fs::private_dir(&runs.root)?;
        let dir = runs.root.join(random_hex()?);
        zajil_platform::fs::private_dir(&dir)?;
        let mut run = Run { dir, out: None };
        zajil_platform::fs::private_dir(&run.tmp())?;
        if let Some(dest) = dest {
            let out = dest.join(format!(".zajil-video-{}", random_hex()?));
            // Written before the folder exists, so a crash never leaves an
            // output folder that no record names.
            write_private(&run.dir.join(OUT_FILE), out.to_string_lossy().as_bytes())?;
            zajil_platform::fs::private_dir(&out)?;
            run.out = Some(out);
        }
        Ok(run)
    }

    fn tmp(&self) -> PathBuf {
        self.dir.join("tmp")
    }

    /// Where yt-dlp works: the output folder of a download, else the
    /// run's own folder.
    fn work_dir(&self) -> PathBuf {
        self.out.clone().unwrap_or_else(|| self.dir.clone())
    }
}

impl Run {
    /// Removes the run's folders now. Blocking; the tree has ended.
    fn close(mut self) {
        let (dir, out) = (std::mem::take(&mut self.dir), self.out.take());
        remove_run(&dir, out.as_deref());
    }
}

fn remove_run(dir: &Path, out: Option<&Path>) {
    if let Some(out) = out {
        let _ = std::fs::remove_dir_all(out);
    }
    if !dir.as_os_str().is_empty() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        // Only when a download future is dropped mid-way (a finished run is
        // closed): in async code, so the removal waits on a blocking thread.
        // The tree, dropped with the same future, was killed first.
        let (dir, out) = (std::mem::take(&mut self.dir), self.out.take());
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => drop(runtime.spawn_blocking(move || remove_run(&dir, out.as_deref()))),
            Err(_) => remove_run(&dir, out.as_deref()),
        }
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = zajil_platform::fs::create_private(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// The cookies of one link, as the Netscape file yt-dlp reads: for the
/// link's host only (never its subdomains), sent only over HTTPS when the
/// link is HTTPS, kept for a day (yt-dlp drops cookies without expiry).
/// Cookies whose name or value could break a line are left out.
pub fn netscape_cookies(url: &reqwest::Url, header: &str, now_unix: u64) -> String {
    let host = url.host_str().unwrap_or_default();
    let secure = if url.scheme() == "https" {
        "TRUE"
    } else {
        "FALSE"
    };
    let expiry = now_unix.saturating_add(24 * 60 * 60);
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    for pair in header.split(';') {
        let Some((name, value)) = pair.trim().split_once('=') else {
            continue;
        };
        let plain = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_graphic() && b != b';');
        if !plain(name) || !(value.is_empty() || plain(value)) {
            continue;
        }
        out.push_str(&format!(
            "{host}\tFALSE\t/\t{secure}\t{expiry}\t{name}\t{value}\n"
        ));
    }
    out
}

/// yt-dlp's message, without what a URL in it may hide (its query,
/// fragment and credentials), on one line, at most 500 characters.
pub fn shown_message(text: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        let trimmed =
            word.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '<' | '>' | ','));
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            out.push_str(&crate::queue::shown_url(trimmed));
        } else {
            out.push_str(&word.replace(|c: char| c.is_control(), ""));
        }
    }
    out.chars().take(500).collect()
}

/// Whether an HLS playlist announces content protection that yt-dlp does
/// not catch by itself (ADR-0009, section 6): any key with a method other
/// than none or AES-128 (`SAMPLE-AES`, `SAMPLE-AES-CTR`…), any key format
/// other than plain `identity` (Widevine, PlayReady, FairPlay…), an `skd://`
/// key, or Adobe's `#EXT-X-FAXS-CM`. AES-128 with a plain key is not DRM:
/// yt-dlp decrypts it like a browser.
pub fn hls_is_protected(playlist: &str) -> bool {
    playlist.lines().any(|line| {
        let line = line.trim();
        if line.starts_with("#EXT-X-FAXS-CM") {
            return true;
        }
        let Some(attrs) = line
            .strip_prefix("#EXT-X-KEY:")
            .or_else(|| line.strip_prefix("#EXT-X-SESSION-KEY:"))
        else {
            return false;
        };
        let upper = attrs.to_ascii_uppercase();
        let method = attribute(&upper, "METHOD").unwrap_or_default();
        let keyformat = attribute(attrs, "KEYFORMAT");
        let uri = attribute(attrs, "URI").unwrap_or_default();
        !matches!(method.as_str(), "NONE" | "AES-128")
            || keyformat.is_some_and(|k| k != "identity")
            || uri.to_ascii_lowercase().starts_with("skd://")
    })
}

/// The value of `NAME=value` or `NAME="value"` in an attribute list.
fn attribute(attrs: &str, name: &str) -> Option<String> {
    let mut rest = attrs;
    while !rest.is_empty() {
        let (key, after) = rest.split_once('=')?;
        let (value, next) = if let Some(quoted) = after.strip_prefix('"') {
            let (value, after) = quoted.split_once('"')?;
            (value, after.strip_prefix(',').unwrap_or(after))
        } else {
            match after.split_once(',') {
                Some((value, next)) => (value, next),
                None => (after, ""),
            }
        };
        if key.trim().eq_ignore_ascii_case(name) {
            return Some(value.trim().to_owned());
        }
        rest = next;
    }
    None
}

/// The longest HLS playlist Zajil reads for its DRM check.
const MAX_PLAYLIST: usize = 4 * 1024 * 1024;

/// Whether a message of yt-dlp says the content is protected.
fn says_drm(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("drm") || lower.contains("sample-aes")
}

/// One video download.
#[derive(Debug, Clone, Copy)]
pub struct Download<'a> {
    pub url: &'a str,
    pub choice: &'a Choice,
    /// The formats the probe found: the HLS ones the choice may use are
    /// checked for DRM before yt-dlp runs.
    pub formats: &'a [Format],
    /// The link's own `Cookie` header from the browser, if any.
    pub cookies: Option<&'a str>,
    /// The folder the video goes to.
    pub dest: &'a Path,
    /// Its name, without extension (the page's title, usually).
    pub name: &'a str,
}

/// Runs yt-dlp for Zajil.
pub struct VideoRunner {
    tools: VideoTools,
    runs: Runs,
    /// For Zajil's own requests (the HLS playlists of the DRM check).
    engine: Engine,
}

impl VideoRunner {
    pub fn new(tools: VideoTools, runs: Runs, engine: Engine) -> Self {
        VideoRunner {
            tools,
            runs,
            engine,
        }
    }

    /// Refuses `choice` if an HLS format it may use is protected in a way
    /// yt-dlp would not catch (it would hand the playlist to ffmpeg).
    async fn check_hls(&self, request: &Download<'_>, stop: &Stop) -> Result<(), VideoError> {
        let used = |f: &Format| match request.choice {
            Choice::Format(id) => f.id == *id,
            Choice::Merge { video, audio } => f.id == *video || f.id == *audio,
            Choice::AudioOnly => f.has_audio() || f.acodec.is_none(),
        };
        let engine = self.engine.with_browser(crate::browser::FromBrowser::new(
            request.url,
            request.cookies,
            None,
        ));
        for format in request.formats.iter().filter(|f| f.is_hls() && used(f)) {
            let Some(Hidden(playlist_url)) = &format.url else {
                continue;
            };
            let playlist = engine
                .fetch_text(playlist_url, MAX_PLAYLIST, stop)
                .await
                .map_err(|e| match e {
                    crate::engine::Error::Stopped { .. } => VideoError::Stopped,
                    e => VideoError::Tool(format!(
                        "the HLS playlist could not be read: {}",
                        shown_message(&e.to_string())
                    )),
                })?;
            if hls_is_protected(&playlist) {
                return Err(VideoError::Drm);
            }
        }
        Ok(())
    }

    /// What `url` holds: one video and its formats, or a playlist's items.
    /// Downloads nothing. `cookies` is the link's own `Cookie` header from
    /// the browser, if any (ADR-0005).
    pub async fn probe(
        &self,
        url: &str,
        cookies: Option<&str>,
        stop: &Stop,
    ) -> Result<Probed, VideoError> {
        let url = checked(url)?;
        let mut args = base_args(&self.tools);
        args.extend([Arg::NoPlaylist, Arg::FlatPlaylist, Arg::DumpSingleJson]);
        let output = self
            .run(&url, args, cookies, None, stop, &mut |_| {})
            .await?;
        let raw: Raw = serde_json::from_slice(&output.stdout)
            .map_err(|e| VideoError::Output(format!("unreadable JSON ({e})")))?;
        if raw.kind.as_deref() == Some("playlist") {
            let mut entries = raw.entries;
            entries.truncate(MAX_PLAYLIST_ENTRIES);
            return Ok(Probed::Playlist {
                title: raw.title,
                entries,
            });
        }
        if !raw.formats.is_empty() && raw.formats.iter().all(Format::is_protected) {
            return Err(VideoError::Drm);
        }
        Ok(Probed::Video(Video {
            title: raw.title,
            duration: raw.duration,
            formats: raw
                .formats
                .into_iter()
                .filter(|f| !f.is_protected())
                .collect(),
        }))
    }

    /// Downloads `request.choice` of `request.url` into `request.dest`, as
    /// `request.name` (sanitized, with the extension yt-dlp chose,
    /// numbered if taken). Returns the file.
    pub async fn download(
        &self,
        request: &Download<'_>,
        stop: &Stop,
        on_event: &mut (dyn FnMut(VideoEvent) + Send),
    ) -> Result<PathBuf, VideoError> {
        let Download {
            url,
            choice,
            cookies,
            dest,
            name,
            ..
        } = *request;
        let url = checked(url)?;
        // Checked before any check of the HLS playlists asks for them.
        let choice_args = choice.args()?;
        self.check_hls(request, stop).await?;
        let mut args = base_args(&self.tools);
        args.extend([
            Arg::NoPlaylist,
            Arg::NoMtime,
            Arg::Output,
            Arg::Progress,
            Arg::Newline,
            Arg::ProgressTemplate,
            Arg::PrintFinalPath,
        ]);
        args.extend(choice_args);
        let finish = Finish {
            dest: dest.to_owned(),
            name: name.to_owned(),
            origin: url.origin().ascii_serialization(),
        };
        let output = self
            .run(&url, args, cookies, Some(finish), stop, on_event)
            .await?;
        match output.placed {
            Some(placed) => Ok(placed?),
            None => Err(VideoError::Output("yt-dlp printed no finished file".into())),
        }
    }

    /// One run of yt-dlp, from start to the end of its whole tree.
    async fn run(
        &self,
        url: &reqwest::Url,
        mut args: Vec<Arg>,
        cookies: Option<&str>,
        finish: Option<Finish>,
        stop: &Stop,
        on_event: &mut (dyn FnMut(VideoEvent) + Send),
    ) -> Result<Output, VideoError> {
        if stop.is_requested() {
            return Err(VideoError::Stopped);
        }
        let (runs_root, tools) = (self.runs.root.clone(), self.tools.clone());
        let dest = finish.as_ref().map(|f| f.dest.clone());
        let cookie_text = cookies.map(|header| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            netscape_cookies(url, header, now)
        });
        let url_text = url.to_string();
        // Prepare the run's folders and the cookie file, and start the tree.
        let (run, tree) = tokio::task::spawn_blocking(move || -> io::Result<_> {
            let run = Run::start(&Runs::new(runs_root), dest.as_deref())?;
            if let Some(text) = cookie_text {
                let file = run.dir.join("cookies.txt");
                write_private(&file, text.as_bytes())?;
                args.push(Arg::Cookies(file));
            }
            let mut command = Command::new(&tools.yt_dlp);
            command
                .args(command_line(&args, &url_text))
                .current_dir(run.work_dir())
                .env_clear()
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            for (key, value) in environment(&run) {
                command.env(key, value);
            }
            let tree = zajil_platform::process::spawn(command)?;
            write_private(
                &run.dir.join(RECORD_FILE),
                tree.record().to_line().as_bytes(),
            )?;
            Ok((run, tree))
        })
        .await
        .map_err(io::Error::other)??;
        // Declared last, so a dropped future drops (kills) the tree before
        // the run's folders go.
        let mut tree = tree;
        let killer = tree.killer();
        let stdout = tree.child().stdout.take().expect("piped");
        let stderr = tree.child().stderr.take().expect("piped");

        // Progress comes through a channel from the blocking reader.
        let (send, mut receive) = tokio::sync::mpsc::unbounded_channel::<VideoEvent>();
        let is_download = run.out.is_some();
        let reader =
            tokio::task::spawn_blocking(move || read_output(stdout, stderr, is_download, send));
        let mut reader = std::pin::pin!(reader);
        let stopped = loop {
            tokio::select! {
                Some(event) = receive.recv() => on_event(event),
                done = &mut reader => break (false, done),
                () = stop.requested() => {
                    let _ = killer.kill();
                    break (true, reader.as_mut().await);
                }
            }
        };
        while let Ok(event) = receive.try_recv() {
            on_event(event);
        }
        let (was_stopped, read) = stopped;
        let read = read.map_err(io::Error::other)?;
        // Wait for the whole tree, then place the file and remove the
        // run's folders, all on a blocking thread.
        let finished = tokio::task::spawn_blocking(move || -> io::Result<Output> {
            let status = tree.wait()?;
            drop(tree);
            let mut output = Output {
                stdout: read.json,
                placed: None,
                status_ok: status.success(),
                error: read.error,
            };
            if output.status_ok
                && !was_stopped
                && let (Some(out), Some(printed), Some(finish)) =
                    (&run.out, read.final_path, finish)
            {
                // Before the run's folders go: the file is moved out.
                output.placed = Some(place(out, &printed).and_then(|file| finish.place(&file)));
            }
            run.close();
            Ok(output)
        })
        .await
        .map_err(io::Error::other)??;
        if was_stopped {
            return Err(VideoError::Stopped);
        }
        if !finished.status_ok {
            let message = finished
                .error
                .unwrap_or_else(|| "it failed without a message".into());
            return Err(if says_drm(&message) {
                VideoError::Drm
            } else {
                VideoError::Tool(message)
            });
        }
        Ok(finished)
    }
}

struct Output {
    stdout: Vec<u8>,
    placed: Option<io::Result<PathBuf>>,
    status_ok: bool,
    error: Option<String>,
}

/// Where a finished video goes, and under what name.
struct Finish {
    dest: PathBuf,
    name: String,
    /// The page's origin, for the mark "from the internet" (#90): never its
    /// path or query, which may hold a token.
    origin: String,
}

impl Finish {
    /// Marks `file` as from the internet, then moves it into `dest` as
    /// `<name>.<its extension>`, numbered if that is taken, never over
    /// another file. Blocking.
    fn place(self, file: &Path) -> io::Result<PathBuf> {
        let ext = file
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| {
                !e.is_empty() && e.len() <= 8 && e.bytes().all(|b| b.is_ascii_alphanumeric())
            })
            .ok_or_else(|| io::Error::other("yt-dlp's file has no usable extension"))?;
        let base = crate::sanitize::sanitize_file_name(&self.name);
        let base = if base.is_empty() {
            "video".to_owned()
        } else {
            base
        };
        let wanted = crate::sanitize::sanitize_file_name(&format!("{base}.{ext}"));
        // A file system that cannot keep the mark keeps the file without it.
        let _ = zajil_platform::origin::mark(file, &self.origin);
        for candidate in crate::naming::candidates(&wanted) {
            let target =
                crate::sanitize::confine(&self.dest, &candidate).map_err(io::Error::other)?;
            match zajil_platform::fs::rename_noreplace(file, &target) {
                Ok(()) => {
                    zajil_platform::fs::sync_dir(&self.dest)?;
                    return Ok(target);
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            }
        }
        Err(io::Error::other(format!("no free name for {wanted}")))
    }
}

fn checked(url: &str) -> Result<reqwest::Url, VideoError> {
    crate::queue::check_url(url).map_err(|e| VideoError::Refused(e.to_string()))?;
    let parsed = reqwest::Url::parse(url).map_err(|e| VideoError::Refused(e.to_string()))?;
    // check_url accepts http and https only, so the URL never starts with
    // '-'; checked again here because the argument list relies on it.
    if !matches!(parsed.scheme(), "http" | "https") || url.starts_with('-') {
        return Err(VideoError::Refused("only http and https links".into()));
    }
    Ok(parsed)
}

/// The environment of a run: nothing inherited but what a program needs to
/// start on the system; home and temporary folders inside the run.
fn environment(run: &Run) -> Vec<(String, std::ffi::OsString)> {
    let tmp = run.tmp().into_os_string();
    let mut env: Vec<(String, std::ffi::OsString)> = vec![
        ("HOME".into(), run.dir.clone().into_os_string()),
        ("USERPROFILE".into(), run.dir.clone().into_os_string()),
        ("TMPDIR".into(), tmp.clone()),
        ("TMP".into(), tmp.clone()),
        ("TEMP".into(), tmp),
        ("LANG".into(), "C.UTF-8".into()),
        ("PYTHONUTF8".into(), "1".into()),
        ("PATH".into(), std::ffi::OsString::new()),
    ];
    // Windows programs need these to start at all; absent elsewhere.
    for name in ["SystemRoot", "SYSTEMROOT", "windir"] {
        if let Some(value) = std::env::var_os(name) {
            env.push((name.into(), value));
        }
    }
    env
}

struct ReadOutput {
    json: Vec<u8>,
    final_path: Option<PathBuf>,
    error: Option<String>,
}

/// Reads yt-dlp's output until it closes: the JSON of a probe (bounded),
/// or the progress and final path of a download; and the last error line.
fn read_output(
    stdout: std::process::ChildStdout,
    stderr: std::process::ChildStderr,
    is_download: bool,
    events: tokio::sync::mpsc::UnboundedSender<VideoEvent>,
) -> ReadOutput {
    let errors = std::thread::spawn(move || {
        let mut last = None;
        for line in bounded_lines(stderr) {
            if let Some(rest) = line.strip_prefix("ERROR:") {
                last = Some(shown_message(rest.trim()));
            }
        }
        last
    });
    let mut json = Vec::new();
    let mut final_path = None;
    if is_download {
        for line in bounded_lines(stdout) {
            if let Some(rest) = line.strip_prefix(PROGRESS_PREFIX) {
                if let Some(event) = parse_progress(rest) {
                    let _ = events.send(event);
                }
            } else if !line.trim().is_empty() {
                final_path = Some(PathBuf::from(line.trim_end_matches(['\r', '\n'])));
            }
        }
    } else {
        let _ = stdout.take(MAX_JSON).read_to_end(&mut json);
    }
    let error = errors.join().unwrap_or(None);
    ReadOutput {
        json,
        final_path,
        error,
    }
}

/// Lines of at most [`MAX_LINE`] bytes, lossy UTF-8; a longer line is cut.
fn bounded_lines(from: impl Read) -> impl Iterator<Item = String> {
    let mut reader = BufReader::new(from);
    std::iter::from_fn(move || {
        let mut buf = Vec::new();
        match (&mut reader)
            .take(MAX_LINE as u64)
            .read_until(b'\n', &mut buf)
        {
            Ok(0) | Err(_) => None,
            Ok(_) => {
                if !buf.ends_with(b"\n") {
                    // Cut: drop the rest of this line.
                    let mut rest = Vec::new();
                    let _ = reader.read_until(b'\n', &mut rest);
                }
                Some(String::from_utf8_lossy(&buf).into_owned())
            }
        }
    })
}

/// A number yt-dlp printed (`NA` when unknown), as bytes.
fn number(word: Option<&str>) -> Option<u64> {
    let value: f64 = word?.parse().ok()?;
    (value.is_finite() && value >= 0.0).then(|| value.min(u64::MAX as f64) as u64)
}

fn parse_progress(rest: &str) -> Option<VideoEvent> {
    let mut words = rest.split_whitespace();
    let done = number(words.next())?;
    let total = number(words.next());
    let estimate = number(words.next());
    let speed = number(words.next());
    Some(VideoEvent::Progress {
        done,
        total: total.or(estimate),
        speed,
    })
}

/// Checks the path yt-dlp printed: a file directly inside the run's
/// output folder, not a link. Returns it.
fn place(out: &Path, printed: &Path) -> io::Result<PathBuf> {
    let name = printed
        .file_name()
        .ok_or_else(|| io::Error::other("yt-dlp printed no file name"))?;
    let inside = out.join(name);
    let same = std::fs::canonicalize(printed)
        .ok()
        .zip(std::fs::canonicalize(&inside).ok())
        .is_some_and(|(a, b)| a == b);
    if !same {
        return Err(io::Error::other(
            "yt-dlp's file is not in its output folder",
        ));
    }
    drop(zajil_platform::fs::open_regular_nofollow(&inside, false)?);
    Ok(inside)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_url_is_last_and_alone_after_the_options() {
        let tools = VideoTools {
            yt_dlp: "yt-dlp".into(),
            ffmpeg_dir: "/tools/ffmpeg".into(),
        };
        let line = command_line(&base_args(&tools), "https://example.com/v?a=--exec");
        let n = line.len();
        assert_eq!(line[n - 2], "--");
        assert_eq!(line[n - 1], "https://example.com/v?a=--exec");
        assert_eq!(line.iter().filter(|a| *a == "--").count(), 1);
    }

    #[test]
    fn format_ids_are_plain_words() {
        for good in ["137", "hls-1080p", "dash_video.mp4", "a.b-c_d"] {
            check_format_id(good).unwrap();
        }
        for bad in [
            "",
            "137+140",
            "best/worst",
            "bv*[height<=720]",
            "a,b",
            "-x",
            "--exec=id",
            "a b",
            "ä",
        ] {
            assert!(check_format_id(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn cookies_go_to_the_link_s_host_only_and_nothing_breaks_a_line() {
        let url = reqwest::Url::parse("https://media.example.com/v.mp4?sig=x").unwrap();
        let text = netscape_cookies(
            &url,
            "sid=abc; theme=dark; bad\tname=1; x=y\nz; empty=",
            100,
        );
        assert_eq!(
            text,
            "# Netscape HTTP Cookie File\n\
             media.example.com\tFALSE\t/\tTRUE\t86500\tsid\tabc\n\
             media.example.com\tFALSE\t/\tTRUE\t86500\ttheme\tdark\n\
             media.example.com\tFALSE\t/\tTRUE\t86500\tempty\t\n"
        );
    }

    #[test]
    fn messages_hide_what_urls_carry() {
        let shown = shown_message(
            "Unable to download https://u:p@cdn.example.com/a/b.m3u8?sig=SECRET#x: HTTP 403",
        );
        assert!(
            !shown.contains("SECRET") && !shown.contains("u:p"),
            "{shown}"
        );
        assert!(
            shown.contains("https://cdn.example.com/a/b.m3u8"),
            "{shown}"
        );
    }

    #[test]
    fn progress_lines_with_unknown_numbers() {
        assert_eq!(
            parse_progress(" 1024 NA 4096.0 512.5"),
            Some(VideoEvent::Progress {
                done: 1024,
                total: Some(4096),
                speed: Some(512)
            })
        );
        assert_eq!(parse_progress(" NA NA NA NA"), None);
        assert_eq!(
            parse_progress(" 1e30 NA NA NA"),
            Some(VideoEvent::Progress {
                done: u64::MAX,
                total: None,
                speed: None
            })
        );
    }

    #[test]
    fn hls_protection_is_recognised_and_plain_aes_is_not() {
        let with = |key: &str| format!("#EXTM3U\n#EXT-X-VERSION:7\n{key}\n#EXTINF:1,\na.m4s\n");
        for protected in [
            r#"#EXT-X-KEY:METHOD=SAMPLE-AES,URI="skd://k",KEYFORMAT="com.apple.streamingkeydelivery""#,
            r#"#EXT-X-KEY:METHOD=SAMPLE-AES-CTR,URI="data:x",KEYFORMAT="urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed""#,
            r#"#EXT-X-SESSION-KEY:METHOD=SAMPLE-AES,URI="https://k",KEYFORMAT="com.microsoft.playready""#,
            r#"#EXT-X-KEY:METHOD=AES-128,URI="skd://k""#,
            "#EXT-X-FAXS-CM:abc",
            r#"#EXT-X-KEY:METHOD=SAMPLE-AES,URI="https://k/key""#,
        ] {
            assert!(hls_is_protected(&with(protected)), "{protected}");
        }
        for plain in [
            r#"#EXT-X-KEY:METHOD=AES-128,URI="https://k/key.bin",IV=0x1234"#,
            r#"#EXT-X-KEY:METHOD=AES-128,URI="https://k/key",KEYFORMAT="identity""#,
            "#EXT-X-KEY:METHOD=NONE",
            "",
        ] {
            assert!(!hls_is_protected(&with(plain)), "{plain}");
        }
    }

    #[test]
    fn a_format_s_url_is_never_shown() {
        let format: Format = serde_json::from_str(
            r#"{"format_id": "1", "protocol": "m3u8_native", "url": "https://cdn/x.m3u8?sig=SECRET"}"#,
        )
        .unwrap();
        assert!(format.is_hls());
        assert!(!format!("{format:?}").contains("SECRET"));
    }

    #[test]
    fn drm_formats_are_recognised() {
        let f = |v: serde_json::Value| Format {
            id: "1".into(),
            ext: "mp4".into(),
            width: None,
            height: None,
            fps: None,
            vcodec: None,
            acodec: None,
            filesize: None,
            filesize_approx: None,
            tbr: None,
            has_drm: Some(v),
            protocol: None,
            url: None,
        };
        assert!(f(serde_json::json!(true)).is_protected());
        assert!(f(serde_json::json!("maybe")).is_protected());
        assert!(!f(serde_json::json!(false)).is_protected());
    }
}
