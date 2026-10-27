//! How Zajil runs yt-dlp (ADR-0009, sections 4 to 6), against a stand-in
//! that records what it was given (tests/support/fake_yt_dlp.rs). The
//! downloads of real HLS, DASH and MP4 with the real tools have their own
//! tests.

use std::fs;
use std::path::{Path, PathBuf};

use zajil_core::tools::{Pinned, Tool, ToolError, Toolbox};
use zajil_core::video::{
    Choice, Download, Format, JsRuntime, Probed, Runs, ToolProblem, VideoError, VideoEvent,
    VideoRunner,
};
use zajil_core::{Engine, Options, Stop};

struct Setup {
    _root: tempfile::TempDir,
    control: PathBuf,
    runs: PathBuf,
    dest: PathBuf,
    /// The stand-in, as the tools folder holds it.
    yt_dlp: PathBuf,
    /// A stand-in for QuickJS-NG's `qjs`, installed and pinned: the runner
    /// checks it only when it is the runtime.
    qjs: PathBuf,
    runner: VideoRunner,
}

const VERSION: &str = "2099.01.01";

fn exe(name: &str) -> String {
    format!("{name}{}", if cfg!(windows) { ".exe" } else { "" })
}

/// Puts `bytes` in the tools folder as `name`, as an install leaves it.
fn put_tool(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    use std::io::Write;
    zajil_platform::fs::private_dir(dir).unwrap();
    let path = dir.join(name);
    let mut file = zajil_platform::fs::create_private(&path).unwrap();
    file.write_all(bytes).unwrap();
    zajil_platform::fs::make_executable(&file).unwrap();
    path
}

/// A runner whose yt-dlp is the stand-in, doing `script`. The stand-in and
/// two stand-ins for ffmpeg and ffprobe are in a tools folder of their
/// own, pinned with their SHA-256, and checked before every run like the
/// real tools. The stand-in's instructions are in the ffmpeg folder, which
/// it gets as `--ffmpeg-location`.
fn setup(script: &str) -> Setup {
    let root = tempfile::tempdir().unwrap();
    let tools = root.path().join("data").join("tools");
    let runs = root.path().join("data").join("run");
    let dest = root.path().join("Downloads");
    fs::create_dir_all(&dest).unwrap();
    let fake = fs::read(env!("CARGO_BIN_EXE_fake-yt-dlp")).unwrap();
    let yt_dlp = put_tool(&tools.join("yt-dlp").join(VERSION), &exe("yt-dlp"), &fake);
    let control = tools.join("ffmpeg").join(VERSION);
    put_tool(&control, &exe("ffmpeg"), b"ffmpeg stand-in");
    put_tool(&control, &exe("ffprobe"), b"ffprobe stand-in");
    let qjs = put_tool(
        &tools.join("quickjs").join(VERSION),
        &exe("qjs"),
        b"qjs stand-in",
    );
    let hashes = [
        (Tool::YtDlp, zajil_testserver::sha256_hex(&fake)),
        (
            Tool::Ffmpeg,
            zajil_testserver::sha256_hex(b"ffmpeg stand-in"),
        ),
        (
            Tool::Ffprobe,
            zajil_testserver::sha256_hex(b"ffprobe stand-in"),
        ),
        (Tool::QuickJs, zajil_testserver::sha256_hex(b"qjs stand-in")),
    ];
    fs::write(control.join("script"), script).unwrap();
    let toolbox = Toolbox::with_pins(
        &tools,
        Box::new(move |tool, _| {
            let (_, sha256) = hashes.iter().find(|(t, _)| *t == tool)?;
            Some(Pinned {
                version: VERSION.to_owned(),
                url: "https://example.invalid/tool".to_owned(),
                sha256: sha256.clone(),
                archive: None,
            })
        }),
    );
    let runner = VideoRunner::new(
        toolbox,
        Runs::new(&runs),
        Engine::new(Options::default()).unwrap(),
    );
    Setup {
        _root: root,
        control,
        runs,
        dest,
        yt_dlp,
        qjs,
        runner,
    }
}

/// Formats the probe found, for the downloads of these tests: plain ones
/// (not HLS), so the DRM check fetches nothing.
fn known() -> &'static [Format] {
    static KNOWN: std::sync::OnceLock<Vec<Format>> = std::sync::OnceLock::new();
    KNOWN.get_or_init(|| {
        ["hls-360", "137", "140", "1"]
            .iter()
            .map(|id| {
                serde_json::from_value(serde_json::json!({
                    "format_id": id, "ext": "mp4", "vcodec": "avc1", "acodec": "mp4a"
                }))
                .unwrap()
            })
            .collect()
    })
}

/// What the stand-in was given, run by run.
fn calls(control: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(control.join("calls.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn args(call: &serde_json::Value) -> Vec<String> {
    call["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_owned())
        .collect()
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

const PROBE: &str = r#"{
  "_type": "video", "title": "دورة Rust (Part 2)", "duration": 4.0,
  "http_headers": {"Cookie": "sid=SECRET-HEADER"},
  "formats": [
    {"format_id": "hls-360", "ext": "mp4", "width": 640, "height": 360, "vcodec": "avc1", "acodec": "mp4a",
     "filesize_approx": 1000, "url": "https://cdn.example.com/a.m3u8?sig=SECRET-URL",
     "http_headers": {"Cookie": "sid=SECRET-HEADER"}, "cookies": "sid=SECRET-COOKIE"},
    {"format_id": "drm-1080", "ext": "mp4", "height": 1080, "vcodec": "avc1", "acodec": "none", "has_drm": true}
  ]
}"#;

#[tokio::test]
async fn a_probe_reads_formats_keeps_no_secret_and_skips_protected_ones() {
    let s = setup("probe");
    fs::write(s.control.join("probe.json"), PROBE).unwrap();
    let probed = s
        .runner
        .probe("https://example.com/watch?v=1", None, &Stop::new())
        .await
        .unwrap();
    let Probed::Video(video) = probed else {
        panic!("{probed:?}")
    };
    assert_eq!(video.title, "دورة Rust (Part 2)");
    assert_eq!(video.formats.len(), 1, "the protected format is left out");
    assert_eq!(video.formats[0].id, "hls-360");
    assert!(video.formats[0].has_video() && video.formats[0].has_audio());
    // Nothing of the headers, cookies or format URLs is kept.
    let kept = format!("{video:?}");
    assert!(!kept.contains("SECRET"), "{kept}");
    // The run's folder is gone.
    assert!(names_in(&s.runs).is_empty(), "{:?}", names_in(&s.runs));
}

#[tokio::test]
async fn every_run_ignores_the_user_s_setup_and_puts_the_url_last_after_two_dashes() {
    let s = setup("probe");
    fs::write(s.control.join("probe.json"), PROBE).unwrap();
    let url = "https://example.com/v?x=--exec=touch%20PWNED";
    s.runner.probe(url, None, &Stop::new()).await.unwrap();
    let call = &calls(&s.control)[0];
    let a = args(call);
    for required in [
        "--ignore-config",
        "--no-plugin-dirs",
        "--no-remote-components",
        "--no-cache-dir",
        "--no-js-runtimes",
        "--no-playlist",
    ] {
        assert!(
            a.contains(&required.to_owned()),
            "{required} missing: {a:?}"
        );
    }
    assert_eq!(&a[a.len() - 2..], ["--", url]);
    assert_eq!(a.iter().filter(|x| *x == "--").count(), 1);
    // yt-dlp's own downloader, our ffmpeg folder.
    let pos = a.iter().position(|x| x == "--downloader").unwrap();
    assert_eq!(a[pos + 1], "native");
    let pos = a.iter().position(|x| x == "--ffmpeg-location").unwrap();
    assert_eq!(Path::new(&a[pos + 1]), s.control);
}

/// Every option yt-dlp may get from Zajil. A run that passes anything
/// else fails this test (ADR-0009: an allow list, not a deny list).
const ALLOWED: &[&str] = &[
    "--ignore-config",
    "--no-plugin-dirs",
    "--no-remote-components",
    "--no-cache-dir",
    "--no-js-runtimes",
    "--no-playlist",
    "--flat-playlist",
    "--playlist-end",
    "--no-color",
    "--no-mtime",
    "--progress",
    "--newline",
    "--dump-single-json",
    "--encoding",
    "--downloader",
    "--ffmpeg-location",
    "--cookies",
    "--output",
    "--format",
    "--extract-audio",
    "--audio-format",
    "--print",
    "--progress-template",
    "--",
];

/// The options of a run (each `-…` word before `--`, and not the value
/// of the option before it).
fn options(a: &[String]) -> Vec<String> {
    let takes_value = [
        "--playlist-end",
        "--encoding",
        "--downloader",
        "--ffmpeg-location",
        "--cookies",
        "--output",
        "--format",
        "--audio-format",
        "--print",
        "--progress-template",
    ];
    let mut out = Vec::new();
    let mut skip = false;
    for word in a {
        if skip {
            skip = false;
            continue;
        }
        if word == "--" {
            break;
        }
        assert!(
            word.starts_with('-'),
            "a value without its option: {word:?} in {a:?}"
        );
        skip = takes_value.contains(&word.as_str());
        out.push(word.clone());
    }
    out
}

#[tokio::test]
async fn no_run_passes_an_option_outside_the_allow_list() {
    for (script, choice) in [
        ("probe", None),
        ("download mp4", Some(Choice::Format("hls-360".into()))),
        (
            "download mkv",
            Some(Choice::Merge {
                video: "137".into(),
                audio: "140".into(),
            }),
        ),
        ("download m4a", Some(Choice::AudioOnly)),
    ] {
        let s = setup(script);
        fs::write(s.control.join("probe.json"), PROBE).unwrap();
        match &choice {
            None => drop(
                s.runner
                    .probe("https://example.com/v", Some("sid=1"), &Stop::new())
                    .await
                    .unwrap(),
            ),
            Some(choice) => {
                let request = Download {
                    url: "https://example.com/v",
                    choice,
                    formats: known(),
                    cookies: Some("sid=1"),
                    dest: &s.dest,
                    name: "x",
                };
                s.runner
                    .download(&request, &Stop::new(), &mut |_| {})
                    .await
                    .unwrap();
            }
        }
        for call in calls(&s.control) {
            for option in options(&args(&call)) {
                assert!(
                    ALLOWED.contains(&option.as_str()),
                    "{option} is not allowed ({script})"
                );
            }
        }
    }
}

#[tokio::test]
async fn nothing_from_the_page_becomes_an_option() {
    let s = setup("probe");
    for url in [
        "--exec=id",
        "-o/etc/x",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "https://u:p@example.com/v",
    ] {
        let err = s.runner.probe(url, None, &Stop::new()).await.unwrap_err();
        assert!(matches!(err, VideoError::Refused(_)), "{url}: {err}");
    }
    for id in ["137+140", "best/worst", "--exec", "bv*", "a,b"] {
        let request = Download {
            url: "https://example.com/v",
            choice: &Choice::Format(id.into()),
            formats: known(),
            cookies: None,
            dest: &s.dest,
            name: "x",
        };
        let err = s
            .runner
            .download(&request, &Stop::new(), &mut |_| {})
            .await
            .unwrap_err();
        assert!(matches!(err, VideoError::Refused(_)), "{id}: {err}");
    }
    assert!(
        calls(&s.control).is_empty(),
        "yt-dlp ran for a refused value"
    );
}

#[tokio::test]
async fn the_environment_is_not_inherited_and_home_and_temp_are_the_run_s_own() {
    let s = setup("probe");
    fs::write(s.control.join("probe.json"), PROBE).unwrap();
    s.runner
        .probe("https://example.com/v", None, &Stop::new())
        .await
        .unwrap();
    let call = &calls(&s.control)[0];
    let env: Vec<(String, String)> = call["env"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p[0].as_str().unwrap().to_owned(),
                p[1].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
    // The test's own variables (cargo sets many) never reach yt-dlp.
    assert!(!names.iter().any(|n| n.starts_with("CARGO")), "{names:?}");
    for (name, value) in &env {
        if ["HOME", "USERPROFILE", "TMPDIR", "TMP", "TEMP"].contains(&name.as_str()) {
            assert!(Path::new(value).starts_with(&s.runs), "{name}={value}");
        }
        if name == "PATH" {
            assert!(value.is_empty(), "PATH={value}");
        }
    }
    assert!(
        names.contains(&"ZAJIL_TOOL_RUN"),
        "the tree's token: {names:?}"
    );
}

#[tokio::test]
async fn the_cookie_file_is_private_holds_only_the_link_s_host_and_is_gone_after() {
    let s = setup("probe");
    fs::write(s.control.join("probe.json"), PROBE).unwrap();
    s.runner
        .probe(
            "https://media.example.com/v",
            Some("sid=abc; theme=dark"),
            &Stop::new(),
        )
        .await
        .unwrap();
    let call = &calls(&s.control)[0];
    assert_eq!(call["cookie_private"], true);
    let text = call["cookie_text"].as_str().unwrap();
    assert!(
        text.contains("media.example.com\tFALSE\t/\tTRUE\t"),
        "{text}"
    );
    assert!(
        text.contains("\tsid\tabc\n") && text.contains("\ttheme\tdark\n"),
        "{text}"
    );
    let cookie_file = args(call)
        .windows(2)
        .find(|w| w[0] == "--cookies")
        .map(|w| PathBuf::from(&w[1]))
        .unwrap();
    assert!(cookie_file.starts_with(&s.runs));
    assert!(!cookie_file.exists());
    assert!(names_in(&s.runs).is_empty());
}

#[tokio::test]
async fn a_download_takes_the_page_s_title_safely_and_never_replaces_a_file() {
    let s = setup("download mp4");
    fs::write(s.dest.join("دورة Rust (Part 2).mp4"), b"mine").unwrap();
    let mut events = Vec::new();
    let request = Download {
        url: "https://example.com/v",
        choice: &Choice::Format("hls-360".into()),
        formats: known(),
        cookies: None,
        dest: &s.dest,
        name: "../دورة Rust (Part 2)",
    };
    let file = s
        .runner
        .download(&request, &Stop::new(), &mut |e| events.push(e))
        .await
        .unwrap();
    // The runner gives the file's real path: on macOS the temporary folder
    // is under /private/var (/var is a link), and on Windows its long name
    // replaces the short one (RUNNER~1). Compare real paths.
    let real = |p: &Path| dunce::canonicalize(p).unwrap();
    assert_eq!(
        real(&file),
        real(&s.dest).join("دورة Rust (Part 2) (1).mp4")
    );
    assert_eq!(fs::read(&file).unwrap(), vec![7u8; 1000]);
    assert_eq!(
        fs::read(s.dest.join("دورة Rust (Part 2).mp4")).unwrap(),
        b"mine"
    );
    assert_eq!(
        events.last(),
        Some(&VideoEvent::Progress {
            done: 1000,
            total: Some(1000),
            speed: None
        })
    );
    // Only the two files: no output folder, nothing in the runs.
    assert_eq!(names_in(&s.dest).len(), 2, "{:?}", names_in(&s.dest));
    assert!(names_in(&s.runs).is_empty());
    // yt-dlp worked in a private folder next to the destination, under a
    // fixed name.
    let call = &calls(&s.control)[0];
    // Gone now, so not resolved: under the destination as given or real.
    let cwd = Path::new(call["cwd"].as_str().unwrap());
    assert!(
        cwd.starts_with(&s.dest) || cwd.starts_with(real(&s.dest)),
        "{cwd:?}"
    );
    let a = args(call);
    let pos = a.iter().position(|x| x == "--output").unwrap();
    assert_eq!(a[pos + 1], "media.%(ext)s");
}

#[tokio::test]
async fn a_name_that_came_with_an_extension_gets_only_yt_dlp_s() {
    for (name, wanted) in [
        ("Talk.mkv", "Talk.mp4"),
        ("Talk.MP4", "Talk.mp4"),
        ("Talk", "Talk.mp4"),
        // Not a video's extension: part of the name.
        ("Rust v1.2", "Rust v1.2.mp4"),
    ] {
        let s = setup("download mp4");
        let request = Download {
            url: "https://example.com/v",
            choice: &Choice::Format("hls-360".into()),
            formats: known(),
            cookies: None,
            dest: &s.dest,
            name,
        };
        let file = s
            .runner
            .download(&request, &Stop::new(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(file.file_name().unwrap().to_str(), Some(wanted), "{name}");
    }
}

#[tokio::test]
async fn a_merge_s_progress_goes_on_from_the_picture_to_the_sound() {
    let sized = |id: &str, size: Option<u64>| -> Format {
        serde_json::from_value(serde_json::json!({
            "format_id": id, "ext": "mp4", "vcodec": "avc1", "acodec": "mp4a", "filesize": size,
        }))
        .unwrap()
    };
    let choice = Choice::Merge {
        video: "137".into(),
        audio: "140".into(),
    };
    // The page gave no size: the whole grows as each part says its own.
    // The page gave both: the whole is known from the start.
    for (formats, first_total) in [
        (vec![sized("137", None), sized("140", None)], 1000),
        (
            vec![sized("137", Some(1000)), sized("140", Some(200))],
            1200,
        ),
    ] {
        let s = setup("download-parts");
        let request = Download {
            url: "https://example.com/v",
            choice: &choice,
            formats: &formats,
            cookies: None,
            dest: &s.dest,
            name: "x",
        };
        let mut seen = Vec::new();
        s.runner
            .download(
                &request,
                &Stop::new(),
                &mut |VideoEvent::Progress { done, total, .. }| seen.push((done, total)),
            )
            .await
            .unwrap();
        assert_eq!(seen.first(), Some(&(500, Some(first_total))), "{seen:?}");
        assert_eq!(seen.last(), Some(&(1200, Some(1200))), "{seen:?}");
        assert!(seen.windows(2).all(|w| w[1].0 >= w[0].0), "{seen:?}");
    }
}

#[tokio::test]
async fn a_file_printed_outside_the_output_folder_is_refused() {
    let s = setup("escape");
    let request = Download {
        url: "https://example.com/v",
        choice: &Choice::Format("1".into()),
        formats: known(),
        cookies: None,
        dest: &s.dest,
        name: "x",
    };
    let err = s
        .runner
        .download(&request, &Stop::new(), &mut |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, VideoError::Io(_)), "{err}");
    assert!(names_in(&s.dest).is_empty(), "{:?}", names_in(&s.dest));
}

#[tokio::test]
async fn yt_dlp_s_errors_hide_link_secrets_and_drm_is_its_own_error() {
    let s = setup(
        "fail Unable to download https://u:p@cdn.example.com/a.m3u8?sig=SECRET: HTTP Error 403",
    );
    let err = s
        .runner
        .probe("https://example.com/v", None, &Stop::new())
        .await
        .unwrap_err();
    let shown = err.to_string();
    assert!(matches!(err, VideoError::Tool(_)), "{shown}");
    assert!(
        !shown.contains("SECRET") && !shown.contains("u:p"),
        "{shown}"
    );
    assert!(shown.contains("HTTP Error 403"), "{shown}");

    let s = setup("fail [generic] This video is DRM protected");
    let err = s
        .runner
        .probe("https://example.com/v", None, &Stop::new())
        .await
        .unwrap_err();
    assert!(matches!(err, VideoError::Drm), "{err}");

    let s = setup("probe");
    fs::write(
        s.control.join("probe.json"),
        r#"{"title": "x", "formats": [{"format_id": "1", "has_drm": true}]}"#,
    )
    .unwrap();
    let err = s
        .runner
        .probe("https://example.com/v", None, &Stop::new())
        .await
        .unwrap_err();
    assert!(matches!(err, VideoError::Drm), "{err}");
}

#[tokio::test]
async fn a_page_with_no_format_for_its_cookies_is_asked_once_more_without_them() {
    let s = setup("signed-in");
    fs::write(s.control.join("probe.json"), PROBE).unwrap();
    let (probed, kept) = s
        .runner
        .probe_with_or_without_cookies(
            "https://www.youtube.com/watch?v=abc",
            Some("SID=x"),
            &Stop::new(),
        )
        .await
        .unwrap();
    assert!(matches!(probed, Probed::Video(_)));
    assert!(!kept, "the download must go without the cookies too");
    let runs = calls(&s.control);
    assert_eq!(runs.len(), 2);
    assert!(args(&runs[0]).contains(&"--cookies".to_owned()));
    assert!(!args(&runs[1]).contains(&"--cookies".to_owned()));
}

#[tokio::test]
async fn cookies_that_work_are_kept_and_another_error_is_not_asked_again() {
    let s = setup("probe");
    fs::write(s.control.join("probe.json"), PROBE).unwrap();
    let (_, kept) = s
        .runner
        .probe_with_or_without_cookies("https://example.com/v", Some("sid=x"), &Stop::new())
        .await
        .unwrap();
    assert!(kept);
    assert_eq!(calls(&s.control).len(), 1);

    let s = setup("fail Sign in to confirm your age");
    let err = s
        .runner
        .probe_with_or_without_cookies("https://example.com/v", Some("sid=x"), &Stop::new())
        .await
        .unwrap_err();
    assert!(matches!(err, VideoError::Tool(_)), "{err}");
    assert_eq!(calls(&s.control).len(), 1);

    // With no cookies, there is nothing to try without.
    let s = setup("fail [youtube] abc: Requested format is not available");
    s.runner
        .probe_with_or_without_cookies("https://example.com/v", None, &Stop::new())
        .await
        .unwrap_err();
    assert_eq!(calls(&s.control).len(), 1);
}

#[tokio::test]
async fn what_a_failure_of_yt_dlp_s_comes_to_is_read_from_its_words() {
    let cases = [
        (
            "fail [youtube] abc: Requested format is not available. Use --list-formats",
            Some(ToolProblem::NoFormats),
        ),
        (
            "fail [youtube] abc: Sign in to confirm your age. This video may be inappropriate",
            Some(ToolProblem::SignIn),
        ),
        (
            "fail [youtube] abc: Sign in to confirm you’re not a bot. Use --cookies-from-browser",
            Some(ToolProblem::SignIn),
        ),
        (
            "fail [youtube] abc: Private video. Sign in if you've been granted access",
            Some(ToolProblem::Private),
        ),
        (
            "fail Unsupported URL: https://example.com/v?token=SECRET",
            Some(ToolProblem::Unsupported),
        ),
        ("fail Unable to download webpage: HTTP Error 500", None),
    ];
    for (script, problem) in cases {
        let s = setup(script);
        let err = s
            .runner
            .probe("https://example.com/v", None, &Stop::new())
            .await
            .unwrap_err();
        assert_eq!(err.problem(), problem, "{script}");
        assert!(!err.to_string().contains("SECRET"), "{err}");
    }

    // YouTube's challenge went unsolved: only a warning says so, before
    // the error that names what followed. That warning is the cause.
    let s = setup("signed-in");
    let err = s
        .runner
        .probe(
            "https://www.youtube.com/watch?v=abc",
            Some("SID=x"),
            &Stop::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.problem(), Some(ToolProblem::Challenge));
    let shown = err.to_string();
    assert!(
        shown.contains("n challenge solving failed")
            && shown.contains("Requested format is not available"),
        "{shown}"
    );
}

#[tokio::test]
async fn a_page_yt_dlp_cannot_read_is_asked_by_its_video_s_own_link() {
    let page = "https://news.example.com/page/1";
    let media = "https://cdn.example.com/v.mp4";
    let s = setup("page-unsupported");
    fs::write(s.control.join("probe.json"), PROBE).unwrap();
    let sent = s
        .runner
        .probe_sent(page, Some(media), Some("sid=x"), &Stop::new())
        .await
        .unwrap();
    assert!(matches!(sent.probed, Probed::Video(_)));
    assert_eq!(sent.url, media);
    assert!(!sent.cookies_kept);
    let runs = calls(&s.control);
    assert_eq!(runs.len(), 2);
    assert!(args(&runs[0]).contains(&"--cookies".to_owned()));
    // The page's cookies never go to the video's own server.
    assert!(!args(&runs[1]).contains(&"--cookies".to_owned()));
    assert_eq!(args(&runs[1]).last().map(String::as_str), Some(media));

    // No link of its own: the page's failure.
    let s = setup("page-unsupported");
    let err = s
        .runner
        .probe_sent(page, None, None, &Stop::new())
        .await
        .unwrap_err();
    assert_eq!(err.problem(), Some(ToolProblem::Unsupported));

    // A page that wants its user signed in is not read another way.
    let s = setup("fail Sign in to confirm your age");
    s.runner
        .probe_sent(page, Some(media), None, &Stop::new())
        .await
        .unwrap_err();
    assert_eq!(calls(&s.control).len(), 1);

    // A page it reads: the page, as before.
    let s = setup("probe");
    fs::write(s.control.join("probe.json"), PROBE).unwrap();
    let sent = s
        .runner
        .probe_sent(
            "https://example.com/v",
            Some(media),
            Some("sid=x"),
            &Stop::new(),
        )
        .await
        .unwrap();
    assert_eq!(sent.url, "https://example.com/v");
    assert!(sent.cookies_kept);
}

#[tokio::test]
async fn a_stopped_run_ends_its_tree_and_leaves_nothing() {
    let s = setup("hang");
    let stop = Stop::new();
    let watcher = {
        let (stop, started) = (stop.clone(), s.control.join("started"));
        tokio::spawn(async move {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !started.exists() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the stand-in never started"
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            stop.request();
        })
    };
    let request = Download {
        url: "https://example.com/v",
        choice: &Choice::Format("1".into()),
        formats: known(),
        cookies: Some("sid=abc"),
        dest: &s.dest,
        name: "x",
    };
    let err = s
        .runner
        .download(&request, &stop, &mut |_| {})
        .await
        .unwrap_err();
    watcher.await.unwrap();
    assert!(matches!(err, VideoError::Stopped), "{err}");
    assert!(names_in(&s.runs).is_empty(), "{:?}", names_in(&s.runs));
    assert!(names_in(&s.dest).is_empty(), "{:?}", names_in(&s.dest));
}

#[tokio::test]
async fn a_playlist_lists_its_entries() {
    let s = setup("probe");
    fs::write(
        s.control.join("probe.json"),
        r#"{"_type": "playlist", "title": "قائمة", "entries": [
            {"url": "https://example.com/1", "title": "one"},
            {"url": "https://example.com/2", "title": "two"}]}"#,
    )
    .unwrap();
    let probed = s
        .runner
        .probe("https://example.com/list", None, &Stop::new())
        .await
        .unwrap();
    let Probed::Playlist { title, entries } = probed else {
        panic!("{probed:?}")
    };
    assert_eq!(title, "قائمة");
    assert_eq!(entries.len(), 2);
    let a = args(&calls(&s.control)[0]);
    assert!(a.contains(&"--flat-playlist".to_owned()));
    // yt-dlp itself stops at the limit: a channel is not read whole first.
    let at = a.iter().position(|x| x == "--playlist-end").unwrap();
    assert_eq!(a[at + 1], "1000");
}

#[tokio::test]
async fn a_playlist_inside_a_playlist_is_told_apart_from_a_video() {
    let s = setup("probe");
    fs::write(
        s.control.join("probe.json"),
        r#"{"_type": "playlist", "title": "Playlists", "entries": [
            {"_type": "url", "ie_key": "YoutubeTab", "url": "https://www.youtube.com/playlist?list=PLx", "title": "p"},
            {"_type": "url", "ie_key": "Youtube", "url": "https://www.youtube.com/watch?v=abc", "title": "v"},
            {"_type": "playlist", "webpage_url": "https://www.youtube.com/playlist?list=PLy", "title": "opened"},
            {"url": "https://example.com/playlist?list=1", "title": "by its link"}]}"#,
    )
    .unwrap();
    let probed = s
        .runner
        .probe("https://www.youtube.com/@c/playlists", None, &Stop::new())
        .await
        .unwrap();
    let Probed::Playlist { entries, .. } = probed else {
        panic!("{probed:?}")
    };
    let playlist: Vec<bool> = entries.iter().map(|e| e.is_playlist()).collect();
    assert_eq!(playlist, [true, false, true, true]);
    assert_eq!(
        entries[2].url.as_deref(),
        Some("https://www.youtube.com/playlist?list=PLy")
    );
}

#[tokio::test]
async fn a_download_dropped_mid_way_ends_its_tree_and_leaves_nothing() {
    // Dropping the future (a closed window, a select! that moved on) is not
    // a stop: nothing gets the chance to clean up in order. The tree dies
    // with the future, and the folders go on a blocking thread.
    let s = setup("hang");
    let started = s.control.join("started");
    let request = Download {
        url: "https://example.com/v",
        choice: &Choice::Format("1".into()),
        formats: known(),
        cookies: Some("sid=abc"),
        dest: &s.dest,
        name: "x",
    };
    let stop = Stop::new();
    let mut on_event = |_| {};
    let download = s.runner.download(&request, &stop, &mut on_event);
    let dropped = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        tokio::pin!(download);
        loop {
            tokio::select! {
                _ = &mut download => panic!("the stand-in hangs until it is killed"),
                () = tokio::time::sleep(std::time::Duration::from_millis(10)) => {
                    if started.exists() {
                        break;
                    }
                }
            }
        }
    })
    .await;
    assert!(dropped.is_ok(), "the stand-in never started");
    // The removal runs on a blocking thread: wait for it, with a margin.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !(names_in(&s.runs).is_empty() && names_in(&s.dest).is_empty()) {
        assert!(
            std::time::Instant::now() < deadline,
            "left: {:?} {:?}",
            names_in(&s.runs),
            names_in(&s.dest)
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn leftovers_of_a_crash_are_ended_and_removed_at_the_next_start() {
    // A run as a crash leaves it: its folder, its cookie file, its output
    // folder, and the record of a tree that still runs.
    let s = setup("hang");
    let run = s.runs.join("0123456789abcdef");
    zajil_platform::fs::private_dir(&run).unwrap();
    fs::write(run.join("cookies.txt"), "x").unwrap();
    let out = s.dest.join(".zajil-video-0123456789abcdef");
    fs::create_dir_all(&out).unwrap();
    fs::write(run.join("out"), out.to_string_lossy().as_bytes()).unwrap();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_fake-yt-dlp"));
    command.arg("--ffmpeg-location").arg(&s.control).env_clear();
    let tree = zajil_platform::process::spawn(command).unwrap();
    fs::write(run.join("tree"), tree.record().to_line()).unwrap();
    let started = s.control.join("started");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !started.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the stand-in never started"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    // The crashed Zajil is gone: its handle on the tree must not be what
    // ends it.
    std::mem::forget(tree);

    let runs = Runs::new(&s.runs);
    tokio::task::spawn_blocking(move || runs.clean_leftovers().unwrap())
        .await
        .unwrap();
    assert!(names_in(&s.runs).is_empty(), "{:?}", names_in(&s.runs));
    assert!(!out.exists());
    // The stand-in was ended by its token: a new start marker never comes
    // and its process is gone (checked by the platform tests); here, the
    // folders it could write to are gone.
}

#[tokio::test]
async fn an_hls_playlist_protected_in_a_way_yt_dlp_misses_is_refused_before_it_runs() {
    // yt-dlp catches FairPlay itself, but hands SAMPLE-AES-CTR with a
    // Widevine key to ffmpeg, which then fails with an unclear message.
    // Zajil reads the playlist first.
    let server = zajil_testserver::TestServer::http();
    server.put(
        "/v/widevine.m3u8",
        zajil_testserver::FileSpec::new(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-KEY:METHOD=SAMPLE-AES-CTR,URI=\"data:x\",\
             KEYFORMAT=\"urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed\"\n#EXTINF:1,\na.m4s\n#EXT-X-ENDLIST\n",
        ),
    );
    server.put(
        "/v/clear.m3u8",
        zajil_testserver::FileSpec::new("#EXTM3U\n#EXTINF:1,\na.m4s\n#EXT-X-ENDLIST\n"),
    );
    let format = |id: &str, path: &str| -> Format {
        serde_json::from_value(serde_json::json!({
            "format_id": id, "ext": "mp4", "vcodec": "avc1", "acodec": "mp4a",
            "protocol": "m3u8_native", "url": server.url(path),
        }))
        .unwrap()
    };
    let formats = [
        format("drm-hls", "/v/widevine.m3u8"),
        format("clear", "/v/clear.m3u8"),
    ];

    let s = setup("download mp4");
    let request = Download {
        url: "https://example.com/v",
        choice: &Choice::Format("drm-hls".into()),
        formats: &formats,
        cookies: None,
        dest: &s.dest,
        name: "x",
    };
    let err = s
        .runner
        .download(&request, &Stop::new(), &mut |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, VideoError::Drm), "{err}");
    assert!(
        calls(&s.control).is_empty(),
        "yt-dlp ran for protected content"
    );

    // The clear playlist of the same page downloads.
    let request = Download {
        choice: &Choice::Format("clear".into()),
        ..request
    };
    s.runner
        .download(&request, &Stop::new(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(calls(&s.control).len(), 1);
}

#[tokio::test]
async fn playlists_for_the_drm_check_come_over_https_only_from_trusted_servers() {
    use zajil_testserver::{FileSpec, TestServer, Tls, test_ca_der};
    let server = TestServer::https(Tls::trusted(&["localhost", "127.0.0.1"]), "localhost");
    server.put("/p.m3u8", FileSpec::new("#EXTM3U\n"));
    // The same path as the download engine: its trusted test root works…
    let trusting = Engine::with_test_root(Options::default(), &test_ca_der()).unwrap();
    let text = trusting
        .fetch_text(&server.url("/p.m3u8"), 1024, &Stop::new())
        .await
        .unwrap();
    assert_eq!(text, "#EXTM3U\n");
    // …and the system's roots alone refuse the test certificate.
    let err = Engine::new(Options::default())
        .unwrap()
        .fetch_text(&server.url("/p.m3u8"), 1024, &Stop::new())
        .await
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("certificate"),
        "{err}"
    );
}

#[tokio::test]
async fn a_playlist_larger_than_the_limit_is_not_read_past_it() {
    use zajil_testserver::{FileSpec, TestServer};
    let server = TestServer::http();
    server.put("/big.m3u8", FileSpec::new(vec![b'#'; 64 * 1024]));
    let err = Engine::new(Options::default())
        .unwrap()
        .fetch_text(&server.url("/big.m3u8"), 1024, &Stop::new())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("larger than 1024 bytes"), "{err}");
}

/// Review of the video runner: the tools are checked before every run, not
/// once when the runner is made. A yt-dlp changed after it was installed
/// (or an ffmpeg) is never run.
#[tokio::test]
async fn a_tool_changed_after_it_was_installed_is_never_run() {
    let s = setup("probe");
    fs::write(s.control.join("probe.json"), PROBE).unwrap();
    s.runner
        .probe("https://example.com/v", None, &Stop::new())
        .await
        .unwrap();
    assert_eq!(calls(&s.control).len(), 1);

    // Another file in its place, after the runner was made.
    fs::remove_file(&s.yt_dlp).unwrap();
    put_tool(
        s.yt_dlp.parent().unwrap(),
        &exe("yt-dlp"),
        b"not the pinned file",
    );
    let err = s
        .runner
        .probe("https://example.com/v", None, &Stop::new())
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            VideoError::Tools(ToolError::Mismatch {
                tool: Tool::YtDlp,
                ..
            })
        ),
        "{err}"
    );
    assert_eq!(calls(&s.control).len(), 1, "a changed yt-dlp ran");

    let s = setup("probe");
    let ffmpeg = s.control.join(exe("ffmpeg"));
    fs::remove_file(&ffmpeg).unwrap();
    put_tool(&s.control, &exe("ffmpeg"), b"another ffmpeg");
    let err = s
        .runner
        .probe("https://example.com/v", None, &Stop::new())
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            VideoError::Tools(ToolError::Mismatch {
                tool: Tool::Ffmpeg,
                ..
            })
        ),
        "{err}"
    );
    assert!(
        calls(&s.control).is_empty(),
        "yt-dlp ran with a changed ffmpeg"
    );
}

/// What yt-dlp got as `call`'s environment.
fn env_of(call: &serde_json::Value) -> Vec<(String, String)> {
    call["env"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p[0].as_str().unwrap().to_owned(),
                p[1].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn quickjs_is_checked_before_every_run_and_named_only_to_the_box() {
    let s = setup("probe");
    fs::write(s.control.join("probe.json"), PROBE).unwrap();
    let runner = s.runner.with_js(Some(JsRuntime::QuickJs {
        jsbox: "/app/zajil-jsbox".into(),
    }));
    runner
        .probe("https://example.com/v", None, &Stop::new())
        .await
        .unwrap();
    let call = &calls(&s.control)[0];
    let a = args(call);
    let set = a.iter().position(|x| x == "--js-runtimes").unwrap();
    assert_eq!(a[set + 1], "quickjs:/app/zajil-jsbox");
    assert!(a.iter().position(|x| x == "--no-js-runtimes").unwrap() < set);
    // yt-dlp never gets `qjs` itself: only the box does, through its
    // environment, with the pinned SHA-256 and the run's report folder.
    let qjs = s.qjs.to_string_lossy().into_owned();
    assert!(!a.iter().any(|x| x.contains(&qjs)), "{a:?}");
    let env = env_of(call);
    let get = |name: &str| {
        env.iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| panic!("{name} not given: {env:?}"))
    };
    assert_eq!(Path::new(&get("ZAJIL_QJS")), s.qjs.as_path());
    assert_eq!(
        get("ZAJIL_QJS_SHA256"),
        zajil_testserver::sha256_hex(b"qjs stand-in")
    );
    assert!(Path::new(&get("ZAJIL_JSBOX_REPORT")).starts_with(&s.runs));
    assert!(
        names_in(&s.runs).is_empty(),
        "the run left {:?}",
        names_in(&s.runs)
    );

    // Another `qjs` in its place, after the runner was made: nothing runs.
    fs::remove_file(&s.qjs).unwrap();
    put_tool(s.qjs.parent().unwrap(), &exe("qjs"), b"another qjs");
    let err = runner
        .probe("https://example.com/v", None, &Stop::new())
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            VideoError::Tools(ToolError::Mismatch {
                tool: Tool::QuickJs,
                ..
            })
        ),
        "{err}"
    );
    assert_eq!(calls(&s.control).len(), 1, "yt-dlp ran with a changed qjs");
}

#[tokio::test]
async fn without_quickjs_as_the_runtime_its_file_is_not_needed() {
    // Tools installed before QuickJS-NG came, or Deno found: `qjs` is not
    // checked, and yt-dlp gets nothing of it.
    for js in [None, Some(JsRuntime::Deno("/opt/deno".into()))] {
        let s = setup("probe");
        fs::write(s.control.join("probe.json"), PROBE).unwrap();
        fs::remove_file(&s.qjs).unwrap();
        let runner = s.runner.with_js(js.clone());
        runner
            .probe("https://example.com/v", None, &Stop::new())
            .await
            .unwrap();
        let call = &calls(&s.control)[0];
        let names: Vec<String> = env_of(call).into_iter().map(|(k, _)| k).collect();
        assert!(
            !names.iter().any(|n| n.starts_with("ZAJIL_QJS")),
            "{names:?}"
        );
        let a = args(call);
        match js {
            None => assert!(!a.iter().any(|x| x == "--js-runtimes")),
            Some(_) => assert!(a.iter().any(|x| x == "deno:/opt/deno"), "{a:?}"),
        }
    }
}

/// ADR-0009, section 10, test 8: the cookie file is gone after a run that
/// failed too, and nothing of the run is left.
#[tokio::test]
async fn a_failed_run_leaves_no_cookie_file() {
    let s = setup("fail something went wrong");
    let request = Download {
        url: "https://example.com/v",
        choice: &Choice::Format("1".into()),
        formats: known(),
        cookies: Some("sid=SECRET"),
        dest: &s.dest,
        name: "x",
    };
    let err = s
        .runner
        .download(&request, &Stop::new(), &mut |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, VideoError::Tool(_)), "{err}");
    assert_eq!(
        calls(&s.control)[0]["cookie_text"]
            .as_str()
            .map(|t| t.contains("SECRET")),
        Some(true)
    );
    assert!(names_in(&s.runs).is_empty(), "{:?}", names_in(&s.runs));
    assert!(names_in(&s.dest).is_empty(), "{:?}", names_in(&s.dest));
}

/// Review of the video runner: a choice that is not one the probe found
/// is refused, never downloaded unchecked for DRM.
#[tokio::test]
async fn a_choice_the_probe_did_not_find_is_refused() {
    let s = setup("download mp4");
    for (choice, formats) in [
        (Choice::Format("999".into()), known()),
        (
            Choice::Merge {
                video: "137".into(),
                audio: "999".into(),
            },
            known(),
        ),
        (Choice::AudioOnly, &[][..]),
    ] {
        let request = Download {
            url: "https://example.com/v",
            choice: &choice,
            formats,
            cookies: None,
            dest: &s.dest,
            name: "x",
        };
        let err = s
            .runner
            .download(&request, &Stop::new(), &mut |_| {})
            .await
            .unwrap_err();
        assert!(matches!(err, VideoError::Refused(_)), "{choice:?}: {err}");
    }
    assert!(calls(&s.control).is_empty());
}

/// Review of the video runner: the sweep at the start never follows a link
/// in place of the runs folder, and leaves a live run of another Zajil
/// alone.
#[tokio::test]
async fn the_sweep_follows_no_link_and_leaves_a_live_run_alone() {
    let s = setup("hang");
    let elsewhere = tempfile::tempdir().unwrap();
    fs::create_dir(elsewhere.path().join("precious")).unwrap();
    let linked_root = s.runs.with_file_name("linked-run");
    zajil_platform::fs::symlink_dir(elsewhere.path(), &linked_root)
        .expect("this test must be able to create a symbolic link");
    assert!(Runs::new(&linked_root).clean_leftovers().is_err());
    assert!(elsewhere.path().join("precious").exists());

    // A live run: the stand-in hangs until stopped.
    let stop = Stop::new();
    let request_stop = stop.clone();
    let control = s.control.clone();
    let runner = s.runner;
    let live =
        tokio::spawn(async move { runner.probe("https://example.com/v", None, &stop).await });
    let started = control.join("started");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !started.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the stand-in never started"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let runs = s.runs.clone();
    tokio::task::spawn_blocking(move || Runs::new(&runs).clean_leftovers().unwrap())
        .await
        .unwrap();
    assert_eq!(names_in(&s.runs).len(), 1, "a live run was swept");
    request_stop.request();
    assert!(matches!(live.await.unwrap(), Err(VideoError::Stopped)));
}
