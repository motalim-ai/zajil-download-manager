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
//! - runs only tools checked right before it ([`Toolbox::verified`]: the
//!   pinned files, in folders no other user can change), kept open until
//!   its whole tree has ended;
//! - has a private folder of its own under `<data dir>/run` (its temporary
//!   files, the cookie file when there is one, and a record of its process
//!   tree), removed as soon as the tree has ended; what a crash leaves is
//!   swept by [`Runs::clean_leftovers`], which the program calls at its
//!   start;
//! - writes the video into a folder of its own next to where it goes,
//!   under a fixed name; Zajil names the final file itself.
//!
//! Content protected by DRM is never downloaded (AGENTS.md, section 6).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

use crate::engine::{Engine, Stop};
use crate::tools::{Tool, Toolbox, VerifiedTool};

/// The tools of one run, checked right before it ([`Toolbox::verified`]),
/// and kept until its whole tree has ended: on Windows they stay locked
/// while yt-dlp may start ffmpeg or ffprobe (review of the video runner:
/// they were checked once, when the runner was made).
struct Checked {
    yt_dlp: VerifiedTool,
    ffmpeg: VerifiedTool,
    _ffprobe: VerifiedTool,
    /// `qjs` and its pinned SHA-256, when the runtime is our QuickJS-NG.
    qjs: Option<(VerifiedTool, String)>,
}

impl Checked {
    /// The folder of our ffmpeg and ffprobe, for `--ffmpeg-location`.
    fn ffmpeg_dir(&self) -> PathBuf {
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
    fn weakest(self, other: Option<JsSandbox>) -> JsSandbox {
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
fn js_held(reports: Option<JsSandbox>, system: JsSandbox) -> Option<JsSandbox> {
    match reports {
        Some(reports) => Some(reports.weakest(Some(system))),
        None if !system.files => Some(system),
        None => None,
    }
}

/// The reports `zajil-jsbox` wrote in `dir`: the weakest, or none if it
/// never ran.
fn js_reports(dir: &Path) -> Option<JsSandbox> {
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
    /// The one JavaScript runtime yt-dlp may use, after [`Arg::NoJsRuntimes`].
    JsRuntime(JsRuntime),
    NoPlaylist,
    FlatPlaylist,
    /// A playlist is read up to [`MAX_PLAYLIST_ENTRIES`] items: a channel
    /// or a long list is not paged through to its end first.
    PlaylistEnd,
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
    /// A format spec built by [`Choice::args`] from ids checked by
    /// [`check_format_id`].
    Format(String),
    /// Audio only, as m4a (AAC).
    AudioOnlyM4a,
    /// Prints the final path, once moved, for Zajil to read.
    PrintFinalPath,
    /// Prints progress in a fixed form Zajil reads.
    ProgressTemplate,
    /// Tests only ([`VideoRunner::with_test_root`]): trust the system's
    /// store, where the test sets its authority, instead of the list
    /// bundled in yt-dlp. Never in a shipped build (no `test-support`).
    #[cfg(feature = "test-support")]
    TestNoCertifi,
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
            Arg::JsRuntime(runtime) => {
                add("--js-runtimes");
                let (name, path) = match runtime {
                    JsRuntime::Deno(path) => ("deno:", path),
                    JsRuntime::Node(path) => ("node:", path),
                    JsRuntime::QuickJs { jsbox } => ("quickjs:", jsbox),
                };
                let mut value = std::ffi::OsString::from(name);
                value.push(path.as_os_str());
                out.push(value);
            }
            Arg::NoPlaylist => add("--no-playlist"),
            Arg::FlatPlaylist => add("--flat-playlist"),
            Arg::PlaylistEnd => {
                add("--playlist-end");
                add(&MAX_PLAYLIST_ENTRIES.to_string());
            }
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
            #[cfg(feature = "test-support")]
            Arg::TestNoCertifi => {
                add("--compat-options");
                add("no-certifi");
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

/// What every run gets, before what the run itself needs (and the tools,
/// added by [`VideoRunner::run`] with [`tool_args`]).
fn base_args() -> Vec<Arg> {
    vec![
        Arg::IgnoreConfig,
        Arg::NoPluginDirs,
        Arg::NoRemoteComponents,
        Arg::NoCacheDir,
        // None of yt-dlp's own choosing: only the one Zajil found, if any.
        Arg::NoJsRuntimes,
        Arg::NoColor,
        Arg::EncodingUtf8,
        Arg::DownloaderNative,
    ]
}

/// Where the checked tools of a run are: our ffmpeg's folder, and the one
/// JavaScript runtime Zajil found, if any.
fn tool_args(ffmpeg_dir: PathBuf, js: Option<&JsRuntime>) -> Vec<Arg> {
    let mut args = vec![Arg::FfmpegLocation(ffmpeg_dir)];
    args.extend(js.cloned().map(Arg::JsRuntime));
    args
}

/// Checks a format id from a probe before it goes into `--format`:
/// letters, digits, `_`, `.` and `-` only, so it can never close the
/// quotes of [`format_spec`] or be an operator of yt-dlp's format language.
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

/// The format with id `id` and no other: yt-dlp reads a bare id as one of
/// its keywords first (`all` downloads every format, `mp4` the best of any
/// mp4: review of the video runner), and a site may name its formats so
/// (yt-dlp's generic extractor names a direct file `mp4`). The filter keeps
/// only that id, compared as text (unquoted, `137` would be compared as a
/// number and match nothing), and `b*` then takes it whatever it holds:
/// picture, sound or both.
fn format_spec(id: &str) -> Result<String, VideoError> {
    check_format_id(id)?;
    Ok(format!("b*[format_id='{id}']"))
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
            Choice::Format(id) => vec![Arg::Format(format_spec(id)?)],
            Choice::Merge { video, audio } => vec![Arg::Format(format!(
                "{}+{}",
                format_spec(video)?,
                format_spec(audio)?
            ))],
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
    /// A tool is missing, or is not the checked file (ADR-0009).
    #[error(transparent)]
    Tools(#[from] crate::tools::ToolError),
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

impl Video {
    /// The best choice when the user names none: the highest picture
    /// with sound in one format, or the highest picture-only format with
    /// the best sound-only one when that picture is higher; sound alone
    /// for a page that has no picture. `None` when nothing is usable.
    ///
    /// An MP4 that every player opens (H.264 picture, AAC sound) comes
    /// first when the page has one: VP9, AV1 or Opus would be merged into
    /// MKV or WebM, which QuickTime on macOS does not play.
    pub fn best_choice(&self) -> Option<Choice> {
        let height = |f: &Format| f.height.unwrap_or(0);
        let rate = |f: &Format| f.tbr.unwrap_or(0.0);
        let by_rate = |a: &&Format, b: &&Format| {
            rate(a)
                .partial_cmp(&rate(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        };
        let better = |a: &&Format, b: &&Format| height(a).cmp(&height(b)).then(by_rate(a, b));
        let mp4_picture = |f: &Format| {
            f.vcodec
                .as_deref()
                .is_some_and(|c| c.starts_with("avc1") || c.starts_with("h264"))
        };
        let mp4_sound = |f: &Format| {
            f.acodec.as_deref().is_some_and(|c| c.starts_with("mp4a")) || f.ext == "m4a"
        };
        // The best of `kind`, among those `preferred` if any.
        let top = |kind: &dyn Fn(&Format) -> bool,
                   preferred: &dyn Fn(&Format) -> bool,
                   order: &dyn Fn(&&Format, &&Format) -> std::cmp::Ordering| {
            let of_kind = || self.formats.iter().filter(|f| kind(f));
            of_kind()
                .filter(|f| preferred(f))
                .max_by(|a, b| order(a, b))
                .or_else(|| of_kind().max_by(|a, b| order(a, b)))
        };
        let both = top(
            &|f| f.has_video() && f.has_audio(),
            &|f| mp4_picture(f) && mp4_sound(f),
            &better,
        );
        let picture = top(&|f| f.has_video() && !f.has_audio(), &mp4_picture, &better);
        let sound = top(&|f| f.has_audio() && !f.has_video(), &mp4_sound, &by_rate);
        match (both, picture, sound) {
            (Some(both), Some(picture), Some(sound)) if height(picture) > height(both) => {
                Some(Choice::Merge {
                    video: picture.id.clone(),
                    audio: sound.id.clone(),
                })
            }
            (Some(both), _, _) => Some(Choice::Format(both.id.clone())),
            (None, Some(picture), Some(sound)) => Some(Choice::Merge {
                video: picture.id.clone(),
                audio: sound.id.clone(),
            }),
            (None, None, Some(_)) => Some(Choice::AudioOnly),
            // Formats whose codecs the page does not say (a direct file):
            // the last one, yt-dlp lists the best last.
            _ => self.formats.last().map(|f| Choice::Format(f.id.clone())),
        }
    }
}

/// One item of a playlist, to be probed and downloaded on its own; or a
/// playlist inside it (a channel's «Playlists»), to be opened.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// `playlist`, or `url` for an item yt-dlp did not open.
    #[serde(rename = "_type", default)]
    pub kind: Option<String>,
    /// What reads an item yt-dlp did not open: `YoutubeTab` for a
    /// playlist or a channel's tab.
    #[serde(default)]
    pub ie_key: Option<String>,
    #[serde(default)]
    webpage_url: Option<String>,
}

impl Entry {
    /// A playlist, not a video: it is opened, never downloaded as one.
    pub fn is_playlist(&self) -> bool {
        self.kind.as_deref() == Some("playlist")
            || self.ie_key.as_deref() == Some("YoutubeTab")
            || self.url.as_deref().is_some_and(|url| {
                reqwest::Url::parse(url).is_ok_and(|u| {
                    u.path() == "/playlist" && u.query_pairs().any(|(k, _)| k == "list")
                })
            })
    }
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
    /// Bytes of the whole download so far (a merge's picture and sound
    /// together), and of all of it when known.
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
/// Held (locked) by a live run, so another Zajil never sweeps it.
const LOCK_FILE: &str = "lock";
const COOKIE_FILE: &str = "cookies.txt";

impl Runs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Runs { root: root.into() }
    }

    /// At program start, before any run: ends every process tree a crash
    /// left running, then removes the folders of those runs and their
    /// output folders. Never through a link (the runs folder must be a
    /// private folder of the user's), never a run another Zajil still runs
    /// (its lock is held), and past a run it cannot remove (the first error
    /// is returned at the end). Blocking.
    pub fn clean_leftovers(&self) -> io::Result<()> {
        match zajil_platform::fs::is_private_dir(&self.root) {
            Ok(true) => {}
            Ok(false) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("{} is not a private folder", self.root.display()),
                ));
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err),
        }
        let mut first_error = None;
        for entry in std::fs::read_dir(&self.root)? {
            let result = entry.and_then(|entry| clean_run(&entry.path()));
            if let Err(err) = result {
                first_error.get_or_insert(err);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

/// One leftover run's folder `dir`, unless a live run holds its lock.
fn clean_run(dir: &Path) -> io::Result<()> {
    // Only a folder, never what a link points to.
    if !std::fs::symlink_metadata(dir)?.is_dir() {
        return Ok(());
    }
    match zajil_platform::fs::open_regular_nofollow(&dir.join(LOCK_FILE), true) {
        Ok(lock) => match lock.try_lock() {
            Ok(()) => {}
            // Another Zajil's live run (the app and `zajil`).
            Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
            Err(std::fs::TryLockError::Error(err)) => return Err(err),
        },
        // A run that crashed before its lock existed.
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    if let Ok(line) = read_small(&dir.join(RECORD_FILE))
        && let Some(record) = zajil_platform::process::Record::from_line(&line)
    {
        zajil_platform::process::end_leftover(record)?;
    }
    if let Ok(out) = read_small(&dir.join(OUT_FILE)) {
        let out = PathBuf::from(out.trim_end_matches('\n'));
        // Only an output folder of ours: a `.zajil-video-*` folder.
        if out
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(".zajil-video-"))
            && std::fs::symlink_metadata(&out).is_ok_and(|m| m.is_dir())
        {
            zajil_platform::fs::remove_dir_all_retrying(&out)?;
        }
    }
    zajil_platform::fs::remove_dir_all_retrying(dir)
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
/// a download, its output folder next to the destination. Both are
/// removed by [`Run::close`] once the tree has ended. A `Run` is only ever
/// dropped without that inside [`Running`], which ends the tree first.
struct Run {
    dir: PathBuf,
    out: Option<PathBuf>,
    /// Locked while the run lives: another Zajil's sweep leaves it alone.
    lock: Option<std::fs::File>,
}

impl Run {
    fn start(runs: &Runs, dest: Option<&Path>) -> io::Result<Run> {
        zajil_platform::fs::private_dir(&runs.root)?;
        let dir = runs.root.join(random_hex()?);
        zajil_platform::fs::private_dir(&dir)?;
        let lock = zajil_platform::fs::create_private(&dir.join(LOCK_FILE))?;
        lock.lock()?;
        let mut run = Run {
            dir,
            out: None,
            lock: Some(lock),
        };
        zajil_platform::fs::private_dir(&run.tmp())?;
        if let Some(dest) = dest {
            let out = dest.join(format!(".zajil-video-{}", random_hex()?));
            // Written before the folder exists, so a crash never leaves an
            // output folder that no record names.
            write_private(&run.dir.join(OUT_FILE), out.to_string_lossy().as_bytes())?;
            // A plain folder of its own (it holds no secret; review of the
            // video runner: a private one was refused on FAT32 and exFAT,
            // and made the video private): new, never one that is there or
            // a link.
            std::fs::create_dir(&out)?;
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

    /// Removes the run's folders now, the cookie file first, again for a
    /// moment while an antivirus holds a file. Blocking; the tree has
    /// ended.
    fn close(mut self) -> io::Result<()> {
        let (dir, out) = (std::mem::take(&mut self.dir), self.out.take());
        // The lock is released before its folder goes (Windows cannot
        // remove an open file).
        drop(self.lock.take());
        let cookies = zajil_platform::fs::remove_file_retrying(&dir.join(COOKIE_FILE));
        let out = out.map_or(Ok(()), |out| {
            zajil_platform::fs::remove_dir_all_retrying(&out)
        });
        let dir = zajil_platform::fs::remove_dir_all_retrying(&dir);
        cookies.and(out).and(dir)
    }
}

impl Drop for Run {
    /// Only a run never closed: one whose start failed half-way, on its
    /// blocking thread, before any tree ran. A run with a tree is always
    /// closed, by [`Running`] if not before.
    fn drop(&mut self) {
        if !self.dir.as_os_str().is_empty() {
            let run = Run {
                dir: std::mem::take(&mut self.dir),
                out: self.out.take(),
                lock: self.lock.take(),
            };
            let _ = run.close();
        }
    }
}

/// A run and its tree, from the moment the tree starts. Dropped (a dropped
/// download future), it ends the tree, then removes the run's folders, in
/// that order and on a blocking thread (review of the video runner: the
/// folders could go while the tree still wrote, and the tree was ended on
/// an async thread).
struct Running(Option<(zajil_platform::process::Tree, Run)>);

impl Running {
    fn take(mut self) -> (zajil_platform::process::Tree, Run) {
        self.0.take().expect("taken once")
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let Some((tree, run)) = self.0.take() else {
            return;
        };
        let end = move || {
            drop(tree);
            let _ = run.close();
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => drop(runtime.spawn_blocking(end)),
            Err(_) => end(),
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
        let word: String = word
            .chars()
            .filter(|&c| !c.is_control() && !crate::category::is_unsafe_in_url(c))
            .collect();
        // A link anywhere in the word (`[https://…]`, `url=https://…`):
        // what is before it stays, the link loses its secrets.
        match ["https://", "http://"].iter().find_map(|s| word.find(s)) {
            Some(at) => {
                let (before, link) = word.split_at(at);
                let link = link.trim_end_matches(|c: char| {
                    matches!(c, '"' | '\'' | '(' | ')' | '<' | '>' | ',' | ']' | '}')
                });
                out.push_str(before);
                out.push_str(&crate::queue::shown_url(link));
            }
            None => out.push_str(&word),
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

/// Whether a message of yt-dlp says the content is protected: its own
/// phrases, not "drm" anywhere (a title or a path, "hdrmovie"…).
fn says_drm(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    ["drm protected", "drm-protected", "uses drm", "sample-aes"]
        .iter()
        .any(|phrase| lower.contains(phrase))
}

/// A download's progress as a whole. yt-dlp counts each part from 0 (a
/// merge: the picture, then the sound), so the parts before are added;
/// the whole is the chosen formats' sizes when the page gave them all.
struct Overall {
    /// The parts that ended.
    before: u64,
    done: u64,
    total: Option<u64>,
    expected: Option<u64>,
}

impl Overall {
    fn new(expected: Option<u64>) -> Overall {
        Overall {
            before: 0,
            done: 0,
            total: None,
            expected,
        }
    }

    fn of(&mut self, event: VideoEvent) -> VideoEvent {
        let VideoEvent::Progress { done, total, speed } = event;
        if done < self.done {
            // Back to the start: the part before has ended.
            self.before += self.total.unwrap_or(0).max(self.done);
        }
        self.done = done;
        self.total = total;
        let so_far = self.before + done;
        let known = total.map(|t| self.before + t.max(done));
        VideoEvent::Progress {
            done: so_far,
            total: match (self.expected, known) {
                (Some(expected), known) => Some(expected.max(known.unwrap_or(so_far))),
                (None, known) => known,
            },
            speed,
        }
    }
}

/// The size of what `choice` downloads, when the page gave every part's.
fn expected_size(choice: &Choice, formats: &[Format]) -> Option<u64> {
    let size = |id: &str| {
        formats
            .iter()
            .find(|f| f.id == id)
            .and_then(|f| f.filesize.or(f.filesize_approx))
    };
    match choice {
        Choice::Format(id) => size(id),
        Choice::Merge { video, audio } => Some(size(video)? + size(audio)?),
        Choice::AudioOnly => None,
    }
}

/// yt-dlp's words when nothing but pictures was left to choose from.
fn says_no_format(message: &str) -> bool {
    message.contains("Requested format is not available")
}

/// yt-dlp's words when YouTube's JavaScript challenge went unsolved.
fn says_challenge(message: &str) -> bool {
    message.contains("challenge solving failed")
}

/// What a failure of yt-dlp's comes to, when its words say: the user is
/// told that in their language, and yt-dlp's own text is only the detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProblem {
    /// No format the page offers could be chosen.
    NoFormats,
    /// The site wants the user signed in (an age check, members only, a
    /// bot check).
    SignIn,
    /// The video is private.
    Private,
    /// yt-dlp knows no way to read this site.
    Unsupported,
    /// YouTube's JavaScript challenge went unsolved.
    Challenge,
}

impl VideoError {
    /// What yt-dlp's failure comes to, when its words say.
    pub fn problem(&self) -> Option<ToolProblem> {
        let VideoError::Tool(message) = self else {
            return None;
        };
        let lower = message.to_ascii_lowercase();
        let says = |phrases: &[&str]| phrases.iter().any(|p| lower.contains(p));
        // The cause before what it caused: an unsolved challenge leaves no
        // format, and a private video asks to sign in.
        Some(if says_challenge(message) {
            ToolProblem::Challenge
        } else if says(&["private video", "video is private"]) {
            ToolProblem::Private
        } else if says(&[
            "sign in to confirm",
            "login required",
            "use --cookies",
            "--cookies-from-browser",
            "only available for registered users",
            "members-only",
            "join this channel",
        ]) {
            ToolProblem::SignIn
        } else if says(&["unsupported url"]) {
            ToolProblem::Unsupported
        } else if says_no_format(message) || says(&["no video formats found"]) {
            ToolProblem::NoFormats
        } else {
            return None;
        })
    }
}

/// One video download. Its `Debug` shows neither its link nor its cookies.
#[derive(Clone, Copy)]
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

impl std::fmt::Debug for Download<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Download")
            .field("url", &crate::queue::shown_url(self.url))
            .field("choice", self.choice)
            .field("formats", &self.formats.len())
            .field("cookies", &self.cookies.map(|_| "…"))
            .field("dest", &self.dest)
            .field("name", &self.name)
            .finish()
    }
}

/// Runs yt-dlp for Zajil.
pub struct VideoRunner {
    tools: Toolbox,
    runs: Runs,
    /// For Zajil's own requests (the HLS playlists of the DRM check).
    engine: Engine,
    /// Tests only: a PEM file of the test authority that yt-dlp trusts.
    #[cfg(feature = "test-support")]
    test_root: Option<PathBuf>,
    /// The JavaScript runtime for YouTube's challenges ([`find_js_runtime`]),
    /// or none: then YouTube may fail, and other sites do not need one.
    js: Option<JsRuntime>,
    /// The weakest sandbox `zajil-jsbox` reported so far.
    js_sandbox: std::sync::Mutex<Option<JsSandbox>>,
}

impl VideoRunner {
    /// A runner of the tools in `tools`, each checked again before every
    /// run.
    pub fn new(tools: Toolbox, runs: Runs, engine: Engine) -> Self {
        VideoRunner {
            tools,
            runs,
            engine,
            #[cfg(feature = "test-support")]
            test_root: None,
            js: None,
            js_sandbox: std::sync::Mutex::new(None),
        }
    }

    /// Gives yt-dlp `js` for YouTube's challenges ([`find_js_runtime`]).
    /// With our QuickJS-NG, `qjs` is checked before every run like the
    /// other tools.
    pub fn with_js(mut self, js: Option<JsRuntime>) -> Self {
        self.js = js;
        self
    }

    /// What the sandbox of `zajil-jsbox` held in the runs so far (the
    /// weakest), if it ran: for the log, and to tell the user when it is
    /// not everything (owner's condition: QuickJS-NG runs without it where
    /// the system has none, but that is said).
    pub fn js_sandbox(&self) -> Option<JsSandbox> {
        *self.js_sandbox.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Lets yt-dlp trust the tests' temporary authority (a PEM file), like
    /// [`Engine::with_test_root`]: only with the `test-support` feature,
    /// which no shipped binary enables (checked in CI).
    #[cfg(feature = "test-support")]
    pub fn with_test_root(mut self, ca_pem: PathBuf) -> Self {
        self.test_root = Some(ca_pem);
        self
    }

    /// The run's arguments and the environment it adds: nothing but in
    /// tests that trust their own authority.
    #[cfg_attr(not(feature = "test-support"), allow(unused_mut))]
    fn all_args(&self, mut args: Vec<Arg>) -> (Vec<Arg>, Vec<(String, std::ffi::OsString)>) {
        #[cfg_attr(not(feature = "test-support"), allow(unused_mut))]
        let mut env = Vec::new();
        #[cfg(feature = "test-support")]
        if let Some(ca) = &self.test_root {
            args.push(Arg::TestNoCertifi);
            env.push(("SSL_CERT_FILE".to_owned(), ca.clone().into_os_string()));
        }
        (args, env)
    }

    /// yt-dlp, ffmpeg and ffprobe, and `qjs` when it is the runtime,
    /// checked now.
    async fn checked_tools(&self) -> Result<Checked, VideoError> {
        let qjs = match &self.js {
            Some(JsRuntime::QuickJs { .. }) => Some((
                self.tools.verified(Tool::QuickJs).await?,
                self.tools.sha256(Tool::QuickJs)?,
            )),
            _ => None,
        };
        Ok(Checked {
            yt_dlp: self.tools.verified(Tool::YtDlp).await?,
            ffmpeg: self.tools.verified(Tool::Ffmpeg).await?,
            _ffprobe: self.tools.verified(Tool::Ffprobe).await?,
            qjs,
        })
    }

    /// Refuses `choice` if an HLS format it may use is protected in a way
    /// yt-dlp would not catch (it would hand the playlist to ffmpeg).
    async fn check_hls(&self, request: &Download<'_>, stop: &Stop) -> Result<(), VideoError> {
        let used = |f: &Format| match request.choice {
            Choice::Format(id) => f.id == *id,
            Choice::Merge { video, audio } => f.id == *video || f.id == *audio,
            Choice::AudioOnly => f.has_audio() || f.acodec.is_none(),
        };
        // Never skipped: a choice the probe's formats do not hold could be
        // a protected one nobody checked (review of the video runner).
        let known = |id: &String| request.formats.iter().any(|f| f.id == *id);
        let all_known = match request.choice {
            Choice::Format(id) => known(id),
            Choice::Merge { video, audio } => known(video) && known(audio),
            Choice::AudioOnly => !request.formats.is_empty(),
        };
        if !all_known {
            return Err(VideoError::Refused(
                "the chosen format is not one the probe found".into(),
            ));
        }
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
        let mut args = base_args();
        args.extend([
            Arg::NoPlaylist,
            Arg::FlatPlaylist,
            Arg::PlaylistEnd,
            Arg::DumpSingleJson,
        ]);
        let output = self
            .run(&url, args, cookies, None, stop, &mut |_| {})
            .await?;
        // Up to 64 MiB: read on a blocking thread.
        let stdout = output.stdout;
        let raw: Raw = tokio::task::spawn_blocking(move || serde_json::from_slice(&stdout))
            .await
            .map_err(io::Error::other)?
            .map_err(|e| VideoError::Output(format!("unreadable JSON ({e})")))?;
        if raw.kind.as_deref() == Some("playlist") {
            let mut entries = raw.entries;
            entries.truncate(MAX_PLAYLIST_ENTRIES);
            for entry in &mut entries {
                // A playlist yt-dlp opened (a channel's page) may give
                // only its page.
                if entry.url.is_none() {
                    entry.url = entry.webpage_url.take();
                }
            }
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

    /// [`probe`](Self::probe) with the browser's cookies, and once more
    /// without them when that found no format at all. Signed in to YouTube,
    /// yt-dlp asks only the players that need JavaScript; with no runtime
    /// for it only pictures are left, while the player it asks without
    /// cookies needs none. Also says whether the cookies were kept: the
    /// download must go the same way, or it would meet the same wall.
    pub async fn probe_with_or_without_cookies(
        &self,
        url: &str,
        cookies: Option<&str>,
        stop: &Stop,
    ) -> Result<(Probed, bool), VideoError> {
        match self.probe(url, cookies, stop).await {
            Err(VideoError::Tool(message)) if cookies.is_some() && says_no_format(&message) => {
                Ok((self.probe(url, None, stop).await?, false))
            }
            probed => Ok((probed?, cookies.is_some())),
        }
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
        let mut args = base_args();
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
        let mut overall = Overall::new(expected_size(choice, request.formats));
        let mut on_part = |event| on_event(overall.of(event));
        let output = self
            .run(&url, args, cookies, Some(finish), stop, &mut on_part)
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
        // The tools, checked now and held until the whole tree has ended.
        let checked = self.checked_tools().await?;
        args.extend(tool_args(checked.ffmpeg_dir(), self.js.as_ref()));
        let (mut args, extra_env) = self.all_args(args);
        let yt_dlp = checked.yt_dlp.path().to_owned();
        let qjs = checked
            .qjs
            .as_ref()
            .map(|(qjs, sha256)| (qjs.path().to_owned(), sha256.clone()));
        let uses_qjs = qjs.is_some();
        let runs_root = self.runs.root.clone();
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
        // A `Running` from here: whoever drops it (this task's output, if
        // the future is gone) ends the tree before the folders go.
        let running = tokio::task::spawn_blocking(move || -> io::Result<Running> {
            let run = Run::start(&Runs::new(runs_root), dest.as_deref())?;
            let fail = |run: Run, err: io::Error| {
                let _ = run.close();
                err
            };
            if let Some(text) = cookie_text {
                let file = run.dir.join(COOKIE_FILE);
                if let Err(err) = write_private(&file, text.as_bytes()) {
                    return Err(fail(run, err));
                }
                args.push(Arg::Cookies(file));
            }
            // What `zajil-jsbox` needs: the checked `qjs`, and where to say
            // what its sandbox held.
            let mut js_env = Vec::new();
            if let Some((qjs, sha256)) = qjs {
                let reports = run.dir.join(JS_REPORTS);
                if let Err(err) = zajil_platform::fs::private_dir(&reports) {
                    return Err(fail(run, err));
                }
                js_env.push(("ZAJIL_QJS".to_owned(), qjs.into_os_string()));
                js_env.push(("ZAJIL_QJS_SHA256".to_owned(), sha256.into()));
                js_env.push(("ZAJIL_JSBOX_REPORT".to_owned(), reports.into_os_string()));
            }
            let mut command = Command::new(&yt_dlp);
            command
                .args(command_line(&args, &url_text))
                .current_dir(run.work_dir())
                .env_clear()
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            for (key, value) in environment(&run).into_iter().chain(extra_env).chain(js_env) {
                command.env(key, value);
            }
            let tree = match zajil_platform::process::spawn(command) {
                Ok(tree) => tree,
                Err(err) => return Err(fail(run, err)),
            };
            let running = Running(Some((tree, run)));
            let (tree, run) = running.0.as_ref().expect("just made");
            write_private(
                &run.dir.join(RECORD_FILE),
                tree.record().to_line().as_bytes(),
            )?;
            Ok(running)
        })
        .await
        .map_err(io::Error::other)??;
        let mut running = running;
        let (killer, stdout, stderr, is_download) = {
            let (tree, run) = running.0.as_mut().expect("not taken yet");
            (
                tree.killer(),
                tree.take_stdout().expect("piped"),
                tree.take_stderr().expect("piped"),
                run.out.is_some(),
            )
        };

        // Progress comes through a channel from the blocking reader.
        let (send, mut receive) = tokio::sync::mpsc::unbounded_channel::<VideoEvent>();
        let reader =
            tokio::task::spawn_blocking(move || read_output(stdout, stderr, is_download, send));
        let mut reader = std::pin::pin!(reader);
        let stopped = loop {
            tokio::select! {
                Some(event) = receive.recv() => on_event(event),
                done = &mut reader => break (false, done),
                () = stop.requested() => {
                    // Ending the tree waits for it: on a blocking thread.
                    let killer = killer.clone();
                    let _ = tokio::task::spawn_blocking(move || killer.kill()).await;
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
            let (mut tree, run) = running.take();
            let status = tree.wait();
            drop(tree);
            let status = match status {
                Ok(status) => status,
                Err(err) => {
                    let _ = run.close();
                    return Err(err);
                }
            };
            let mut output = Output {
                stdout: read.json,
                placed: None,
                status_ok: status.success(),
                error: read.error,
                js: if uses_qjs {
                    js_held(
                        js_reports(&run.dir.join(JS_REPORTS)),
                        zajil_platform::sandbox::supported().into(),
                    )
                } else {
                    None
                },
            };
            if output.status_ok
                && !was_stopped
                && let (Some(out), Some(printed), Some(finish)) =
                    (&run.out, read.final_path, finish)
            {
                // Before the run's folders go: the file is moved out.
                output.placed = Some(place(out, &printed).and_then(|file| finish.place(&file)));
            }
            run.close()?;
            Ok(output)
        })
        .await
        .map_err(io::Error::other)??;
        // Until here: on Windows, nobody could change a tool the tree ran.
        drop(checked);
        if let Some(this) = finished.js {
            let mut seen = self.js_sandbox.lock().unwrap_or_else(|e| e.into_inner());
            *seen = Some(this.weakest(*seen));
        }
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
    /// What `zajil-jsbox`'s sandbox held, if it ran.
    js: Option<JsSandbox>,
}

/// The folder of a run where `zajil-jsbox` reports.
const JS_REPORTS: &str = "js-report";

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
        let base = crate::sanitize::sanitize_file_name(without_media_extension(&self.name));
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
                    // The tool made it under `umask 077`: the mode of any
                    // other download.
                    let placed = zajil_platform::fs::open_regular_nofollow(&target, false)?;
                    zajil_platform::fs::make_ordinary(&placed)?;
                    return Ok(target);
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            }
        }
        Err(io::Error::other(format!("no free name for {wanted}")))
    }
}

/// `name` without a video or sound extension at its end: the file gets
/// the one yt-dlp chose, and a name that came with one would end
/// `Title.mkv.mp4`.
fn without_media_extension(name: &str) -> &str {
    const MEDIA: [&str; 15] = [
        "3gp", "aac", "avi", "flac", "flv", "m4a", "m4v", "mkv", "mov", "mp3", "mp4", "ogg",
        "opus", "wav", "webm",
    ];
    match name.trim_end().rsplit_once('.') {
        Some((base, ext))
            if !base.trim().is_empty() && MEDIA.iter().any(|m| ext.eq_ignore_ascii_case(m)) =>
        {
            base
        }
        _ => name,
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
        // A warning that says why, when the error names only what followed
        // (no format left after YouTube's challenge went unsolved).
        let mut cause = None;
        for line in bounded_lines(stderr) {
            if let Some(rest) = line.strip_prefix("ERROR:") {
                last = Some(shown_message(rest.trim()));
            } else if let Some(rest) = line.strip_prefix("WARNING:")
                && cause.is_none()
                && says_challenge(rest)
            {
                cause = Some(shown_message(rest.trim()));
            }
        }
        match (cause, last) {
            (Some(cause), Some(last)) => Some(format!("{cause} / {last}")),
            (cause, last) => last.or(cause),
        }
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
                    // Cut: skip the rest of this line, keeping nothing of it
                    // (review of the video runner: it was read whole).
                    while let Ok(chunk) = reader.fill_buf() {
                        if chunk.is_empty() {
                            break;
                        }
                        match chunk.iter().position(|&b| b == b'\n') {
                            Some(at) => {
                                reader.consume(at + 1);
                                break;
                            }
                            None => {
                                let n = chunk.len();
                                reader.consume(n);
                            }
                        }
                    }
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
        let line = command_line(&base_args(), "https://example.com/v?a=--exec");
        let n = line.len();
        assert_eq!(line[n - 2], "--");
        assert_eq!(line[n - 1], "https://example.com/v?a=--exec");
        assert_eq!(line.iter().filter(|a| *a == "--").count(), 1);
    }

    #[test]
    fn only_the_runtime_zajil_found_goes_to_yt_dlp_after_clearing_its_own() {
        let line = |js: Option<JsRuntime>| {
            let mut args = base_args();
            args.extend(tool_args("/tools/ffmpeg".into(), js.as_ref()));
            command_line(&args, "https://example.com/v")
        };
        // None found: yt-dlp may use none (not even its default, Deno).
        let none = line(None);
        assert!(none.iter().any(|a| a == "--no-js-runtimes"));
        assert!(!none.iter().any(|a| a == "--js-runtimes"));
        for (js, wanted) in [
            (JsRuntime::Deno("/opt/deno".into()), "deno:/opt/deno"),
            (
                JsRuntime::Node("/usr/bin/node".into()),
                "node:/usr/bin/node",
            ),
            (
                JsRuntime::QuickJs {
                    jsbox: "/app/zajil-jsbox".into(),
                },
                "quickjs:/app/zajil-jsbox",
            ),
        ] {
            let args = line(Some(js));
            let clear = args.iter().position(|a| a == "--no-js-runtimes").unwrap();
            let set = args.iter().position(|a| a == "--js-runtimes").unwrap();
            assert!(clear < set);
            assert_eq!(args[set + 1], wanted);
            assert_eq!(args.iter().filter(|a| *a == "--js-runtimes").count(), 1);
        }
    }

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
        // yt-dlp's keywords are ids too: the spec keeps only that id.
        for keyword in ["all", "mergeall", "best", "b", "bv*", "mp4", "M4A", "w"] {
            if check_format_id(keyword).is_ok() {
                assert_eq!(
                    format_spec(keyword).unwrap(),
                    format!("b*[format_id='{keyword}']")
                );
            }
        }
        assert_eq!(
            Choice::Merge {
                video: "137".into(),
                audio: "mp4".into()
            }
            .args()
            .unwrap(),
            vec![Arg::Format(
                "b*[format_id='137']+b*[format_id='mp4']".into()
            )]
        );
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
    fn the_best_choice_prefers_higher_pictures_and_keeps_sound() {
        let f = |id: &str, h: Option<u32>, v: &str, a: &str| -> Format {
            serde_json::from_value(serde_json::json!({
                "format_id": id, "height": h, "vcodec": v, "acodec": a, "tbr": 100.0,
            }))
            .unwrap()
        };
        let video = |formats| Video {
            title: String::new(),
            duration: None,
            formats,
        };
        // A higher picture alone beats a lower one with sound: merged.
        assert_eq!(
            video(vec![
                f("18", Some(360), "avc1", "mp4a"),
                f("137", Some(1080), "avc1", "none"),
                f("140", None, "none", "mp4a")
            ])
            .best_choice(),
            Some(Choice::Merge {
                video: "137".into(),
                audio: "140".into()
            })
        );
        // Picture with sound at the top height: that one.
        assert_eq!(
            video(vec![
                f("22", Some(720), "avc1", "mp4a"),
                f("136", Some(720), "avc1", "none"),
                f("140", None, "none", "mp4a")
            ])
            .best_choice(),
            Some(Choice::Format("22".into()))
        );
        // Sound only.
        assert_eq!(
            video(vec![f("140", None, "none", "mp4a")]).best_choice(),
            Some(Choice::AudioOnly)
        );
        // Unknown codecs (a direct file): the last.
        let mut plain = f("0", None, "x", "x");
        plain.vcodec = None;
        plain.acodec = None;
        assert_eq!(
            video(vec![plain]).best_choice(),
            Some(Choice::Format("0".into()))
        );
        assert_eq!(video(vec![]).best_choice(), None);
    }

    /// A YouTube page: VP9 and AV1 pictures above the H.264 one, Opus
    /// sound with a higher rate than AAC. The default is the MP4 that
    /// every player opens.
    #[test]
    fn the_best_choice_is_an_mp4_when_the_page_has_one() {
        let f = |id: &str, h: Option<u32>, ext: &str, v: &str, a: &str, tbr: f64| -> Format {
            serde_json::from_value(serde_json::json!({
                "format_id": id, "height": h, "ext": ext, "vcodec": v, "acodec": a, "tbr": tbr,
            }))
            .unwrap()
        };
        let page = Video {
            title: String::new(),
            duration: None,
            formats: vec![
                f("18", Some(360), "mp4", "avc1.42001E", "mp4a.40.2", 500.0),
                f("139", None, "m4a", "none", "mp4a.40.5", 49.0),
                f("140", None, "m4a", "none", "mp4a.40.2", 129.0),
                f("251", None, "webm", "none", "opus", 135.0),
                f("251-drc", None, "webm", "none", "opus", 160.0),
                f("137", Some(1080), "mp4", "avc1.640028", "none", 4000.0),
                f("399", Some(1080), "mp4", "av01.0.08M.08", "none", 2000.0),
                f("616", Some(1440), "mp4", "vp09.00.40.08", "none", 9000.0),
            ],
        };
        assert_eq!(
            page.best_choice(),
            Some(Choice::Merge {
                video: "137".into(),
                audio: "140".into()
            })
        );
        // No H.264 at all: the highest picture, with AAC still first.
        let mut other = page.clone();
        other
            .formats
            .retain(|f| !f.vcodec.as_deref().unwrap().starts_with("avc1"));
        assert_eq!(
            other.best_choice(),
            Some(Choice::Merge {
                video: "616".into(),
                audio: "140".into()
            })
        );
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

#[cfg(test)]
mod more_tests {
    use super::*;

    /// AGENTS.md §10.8: a line longer than the limit is cut, and the rest
    /// of it is skipped without being kept (it was read whole).
    #[test]
    fn a_huge_line_is_cut_and_its_rest_is_never_kept() {
        struct Endless {
            left: u64,
            then: &'static [u8],
        }
        impl Read for Endless {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.left > 0 {
                    let n = (buf.len() as u64).min(self.left) as usize;
                    buf[..n].fill(b'a');
                    self.left -= n as u64;
                    return Ok(n);
                }
                let n = buf.len().min(self.then.len());
                buf[..n].copy_from_slice(&self.then[..n]);
                self.then = &self.then[n..];
                Ok(n)
            }
        }
        let lines: Vec<String> = bounded_lines(Endless {
            left: 100 * 1024 * 1024,
            then: b"\nERROR: next\n",
        })
        .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), MAX_LINE);
        assert_eq!(lines[1], "ERROR: next\n");
    }

    #[test]
    fn messages_lose_link_secrets_wherever_the_link_is() {
        for text in [
            "failed [https://cdn.example.com/a?sig=SECRET]",
            "url=https://u:p@cdn.example.com/a?sig=SECRET#frag",
            "(https://cdn.example.com/a?sig=SECRET)",
            "x\u{202E}https://cdn.example.com/a?sig=SECRET",
        ] {
            let shown = shown_message(text);
            assert!(
                !shown.contains("SECRET") && !shown.contains("u:p"),
                "{shown}"
            );
            assert!(!shown.contains('\u{202E}'), "{shown:?}");
            assert!(shown.contains("cdn.example.com/a"), "{shown}");
        }
    }

    #[test]
    fn only_yt_dlp_s_own_drm_phrases_are_drm() {
        assert!(says_drm("This video is DRM protected"));
        assert!(says_drm("The requested format uses SAMPLE-AES"));
        for not_drm in [
            "Unable to download hdrmovie.mp4",
            "HTTP Error 404 at /drm/x",
        ] {
            assert!(!says_drm(not_drm), "{not_drm}");
        }
    }

    #[test]
    fn a_download_s_debug_shows_no_link_secret_nor_cookie() {
        let choice = Choice::Format("18".into());
        let request = Download {
            url: "https://example.com/v?token=SECRET",
            choice: &choice,
            formats: &[],
            cookies: Some("sid=SECRET"),
            dest: Path::new("/tmp"),
            name: "x",
        };
        let shown = format!("{request:?}");
        assert!(!shown.contains("SECRET"), "{shown}");
    }

    /// AGENTS.md §10.1 and §10.7: the file yt-dlp printed must be a plain
    /// file directly in the output folder (a link to elsewhere is
    /// refused), and a long title whose name is taken is numbered within
    /// the limit.
    #[test]
    fn the_printed_file_must_be_a_plain_file_and_long_titles_fit() {
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("out");
        let dest = root.path().join("dest");
        let elsewhere = root.path().join("elsewhere.mp4");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(&elsewhere, b"not yours").unwrap();
        let link = out.join("media.mp4");
        zajil_platform::fs::symlink_file(&elsewhere, &link)
            .expect("this test must be able to create a symbolic link");
        assert!(place(&out, &link).is_err(), "a link was taken");
        std::fs::remove_file(&link).unwrap();
        std::fs::hard_link(&elsewhere, &link).unwrap();
        assert!(place(&out, &link).is_err(), "a hard link was taken");
        std::fs::remove_file(&link).unwrap();
        assert_eq!(std::fs::read(&elsewhere).unwrap(), b"not yours");

        for total in [197, 198, 199, 200] {
            let title = format!(
                "{}{}",
                "ع".repeat((total - 4) / 2),
                "x".repeat((total - 4) % 2)
            );
            let taken = format!("{title}.mp4");
            assert_eq!(taken.len(), total);
            std::fs::write(dest.join(&taken), b"the user's").unwrap();
            let file = out.join("media.mp4");
            std::fs::write(&file, b"video").unwrap();
            let placed = place(&out, &file).unwrap();
            let finish = Finish {
                dest: dest.clone(),
                name: title.clone(),
                origin: "https://example.com".into(),
            };
            let target = finish.place(&placed).unwrap();
            let got = target.file_name().unwrap().to_str().unwrap();
            assert!(got.len() <= crate::sanitize::MAX_NAME_BYTES, "{got}");
            assert!(got.ends_with(".mp4") && got != taken, "{got}");
            assert_eq!(std::fs::read(dest.join(&taken)).unwrap(), b"the user's");
            assert_eq!(std::fs::read(&target).unwrap(), b"video");
        }
    }
}
