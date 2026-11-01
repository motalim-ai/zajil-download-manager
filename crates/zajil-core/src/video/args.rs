//! yt-dlp's arguments, built only from a fixed set: the options, the format
//! asked for, and the environment of its run.

use std::path::PathBuf;

use super::VideoError;
use super::js::JsRuntime;
use super::output::MAX_PLAYLIST_ENTRIES;
use super::runs::Run;

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
    /// The warm-up run ([`VideoRunner::warm_up`]): yt-dlp's verbose header,
    /// which loads every library it has, then a simulated run whose every
    /// request goes, as to a proxy, to this port of `127.0.0.1`, where
    /// Zajil itself drops each connection: nothing leaves the machine.
    WarmUp(u16),
    /// Tests only ([`VideoRunner::with_test_root`]): trust the system's
    /// store, where the test sets its authority, instead of the list
    /// bundled in yt-dlp. Never in a shipped build (no `test-support`).
    #[cfg(feature = "test-support")]
    TestNoCertifi,
}

/// The page the warm-up run "opens": a YouTube video, so YouTube's own
/// code is loaded too; no request for it leaves the machine.
pub(super) const WARM_UP_URL: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";
/// The longest the warm-up may take: about two minutes was seen on a Mac
/// for the first full run of a newly unpacked yt-dlp.
pub(super) const WARM_UP_TIME: std::time::Duration = std::time::Duration::from_secs(300);

/// The progress line Zajil asks for: a fixed word, then numbers or `NA`.
pub(super) const PROGRESS_PREFIX: &str = "zajil-progress";

impl Arg {
    fn push(&self, out: &mut Vec<std::ffi::OsString>) {
        let mut add = |s: &str| out.push(s.into());
        match self {
            Arg::IgnoreConfig => add("--ignore-config"),
            Arg::NoPluginDirs => add("--no-plugin-dirs"),
            Arg::NoRemoteComponents => add("--no-remote-components"),
            Arg::NoCacheDir => add("--no-cache-dir"),
            Arg::NoJsRuntimes => add("--no-js-runtimes"),
            Arg::WarmUp(port) => {
                for word in ["--verbose", "--simulate", "--proxy"] {
                    add(word);
                }
                add(&format!("http://127.0.0.1:{port}/"));
                for word in [
                    "--socket-timeout",
                    "5",
                    "--retries",
                    "0",
                    "--extractor-retries",
                    "0",
                ] {
                    add(word);
                }
            }
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
pub(super) fn base_args() -> Vec<Arg> {
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
pub(super) fn tool_args(ffmpeg_dir: PathBuf, js: Option<&JsRuntime>) -> Vec<Arg> {
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
pub(super) fn format_spec(id: &str) -> Result<String, VideoError> {
    check_format_id(id)?;
    Ok(format!("b*[format_id='{id}']"))
}

/// The environment of a run: nothing inherited but what a program needs to
/// start on the system; home and temporary folders inside the run.
pub(super) fn environment(run: &Run) -> Vec<(String, std::ffi::OsString)> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::Choice;

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
}
