//! How Zajil runs yt-dlp (ADR-0009, sections 4 to 6), against a stand-in
//! that records what it was given (tests/support/fake_yt_dlp.rs). The
//! downloads of real HLS, DASH and MP4 with the real tools have their own
//! tests.

use std::fs;
use std::path::{Path, PathBuf};

use zajil_core::Stop;
use zajil_core::video::{
    Choice, Download, Probed, Runs, VideoError, VideoEvent, VideoRunner, VideoTools,
};

struct Setup {
    _root: tempfile::TempDir,
    control: PathBuf,
    runs: PathBuf,
    dest: PathBuf,
    runner: VideoRunner,
}

/// A runner whose yt-dlp is the stand-in, doing `script`.
fn setup(script: &str) -> Setup {
    let root = tempfile::tempdir().unwrap();
    let control = root.path().join("control");
    let runs = root.path().join("data").join("run");
    let dest = root.path().join("Downloads");
    fs::create_dir_all(&control).unwrap();
    fs::create_dir_all(&dest).unwrap();
    fs::write(control.join("script"), script).unwrap();
    let runner = VideoRunner::new(
        VideoTools {
            yt_dlp: PathBuf::from(env!("CARGO_BIN_EXE_fake-yt-dlp")),
            ffmpeg_dir: control.clone(),
        },
        Runs::new(&runs),
    );
    Setup {
        _root: root,
        control,
        runs,
        dest,
        runner,
    }
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
        cookies: None,
        dest: &s.dest,
        name: "../دورة Rust (Part 2)",
    };
    let file = s
        .runner
        .download(&request, &Stop::new(), &mut |e| events.push(e))
        .await
        .unwrap();
    assert_eq!(file, s.dest.join("دورة Rust (Part 2) (1).mp4"));
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
    assert!(Path::new(call["cwd"].as_str().unwrap()).starts_with(&s.dest));
    let a = args(call);
    let pos = a.iter().position(|x| x == "--output").unwrap();
    assert_eq!(a[pos + 1], "media.%(ext)s");
}

#[tokio::test]
async fn a_file_printed_outside_the_output_folder_is_refused() {
    let s = setup("escape");
    let request = Download {
        url: "https://example.com/v",
        choice: &Choice::Format("1".into()),
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
async fn a_stopped_run_ends_its_tree_and_leaves_nothing() {
    let s = setup("hang");
    let stop = Stop::new();
    let watcher = {
        let (stop, started) = (stop.clone(), s.control.join("started"));
        tokio::spawn(async move {
            while !started.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            stop.request();
        })
    };
    let request = Download {
        url: "https://example.com/v",
        choice: &Choice::Format("1".into()),
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
    while !started.exists() {
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
