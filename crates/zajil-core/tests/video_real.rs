//! Video downloads with the real yt-dlp and our real ffmpeg (ADR-0009,
//! section 8; ROADMAP M6): MP4, HLS, DASH with separate picture and sound,
//! a page with `<video>`, audio only, DRM, cookies and TLS. Every file is
//! made for the test by `ffmpeg-testgen` and served by the local test
//! server; nothing reaches the internet.
//!
//! The tools come from the CI (`.github/workflows/ci.yml`), through:
//! - `ZAJIL_TEST_YT_DLP`: the pinned yt-dlp program;
//! - `ZAJIL_TEST_FFMPEG_DIR`: the folder of the distributed ffmpeg and
//!   ffprobe;
//! - `ZAJIL_TEST_FFMPEG_TESTGEN`: the folder of `ffmpeg-testgen`.
//!
//! On the CI a missing tool fails these tests, except `ffmpeg-testgen`
//! while the cache has none yet (`ZAJIL_TEST_NO_TESTGEN`, set by ci.yml
//! with a warning: ffmpeg.yml builds it). On a developer's machine
//! without them, each test says so on stderr and returns.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use zajil_core::tools::{Archive, Pinned, Tool, Toolbox, tree_sha256};
use zajil_core::video::{Choice, Download, Probed, Runs, VideoError, VideoRunner};
use zajil_core::{Engine, Options, Stop};
use zajil_testserver::{FileSpec, TestServer, Tls, test_ca_der};

struct Tools {
    yt_dlp: PathBuf,
    ffmpeg_dir: PathBuf,
    testgen: PathBuf,
}

fn tools() -> Option<Tools> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    match (
        var("ZAJIL_TEST_YT_DLP"),
        var("ZAJIL_TEST_FFMPEG_DIR"),
        var("ZAJIL_TEST_FFMPEG_TESTGEN"),
    ) {
        (Some(yt_dlp), Some(ffmpeg_dir), Some(testgen)) => Some(Tools {
            yt_dlp,
            ffmpeg_dir,
            testgen,
        }),
        (Some(_), Some(_), None) if std::env::var_os("ZAJIL_TEST_NO_TESTGEN").is_some() => {
            eprintln!(
                "NOT RUN: ffmpeg-testgen is not in the CI's cache yet (ffmpeg.yml builds it)"
            );
            None
        }
        _ if std::env::var_os("CI").is_some() => {
            panic!(
                "the video tools are missing on the CI: ZAJIL_TEST_YT_DLP, ZAJIL_TEST_FFMPEG_DIR and ZAJIL_TEST_FFMPEG_TESTGEN must be set"
            )
        }
        _ => {
            eprintln!(
                "NOT RUN: set ZAJIL_TEST_YT_DLP, ZAJIL_TEST_FFMPEG_DIR and ZAJIL_TEST_FFMPEG_TESTGEN to run the real video tests"
            );
            None
        }
    }
}

/// The real tools, copied into a tools folder of `root` as an install
/// leaves them, pinned with their own SHA-256: the runner checks them
/// before every run like any tools. A yt-dlp unpacked from its zip (macOS
/// and Windows) is copied with its whole folder, and pinned with the tree
/// SHA-256 of what it holds.
fn toolbox(tools: &Tools, root: &Path) -> Toolbox {
    use std::io::Write;
    const VERSION: &str = "2099.01.01";
    let dir = root.join("tools");
    let mut hashes = Vec::new();
    let unpacked = tools.yt_dlp.parent().unwrap();
    let archive = unpacked.join("_internal").is_dir().then(|| {
        let folder = unpacked.file_name().unwrap().to_str().unwrap().to_owned();
        let program = tools
            .yt_dlp
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let into = dir.join("yt-dlp").join(VERSION).join(&folder);
        let files = copy_tree(unpacked, &into, &program);
        Archive {
            folder,
            program,
            tree_sha256: tree_sha256(&files),
        }
    });
    for (tool, from, folder) in [
        (Tool::YtDlp, tools.yt_dlp.clone(), "yt-dlp"),
        (Tool::Ffmpeg, exe(&tools.ffmpeg_dir, "ffmpeg"), "ffmpeg"),
        (Tool::Ffprobe, exe(&tools.ffmpeg_dir, "ffprobe"), "ffmpeg"),
    ] {
        let bytes = fs::read(&from).unwrap();
        if tool == Tool::YtDlp && archive.is_some() {
            hashes.push((tool, zajil_testserver::sha256_hex(&bytes)));
            continue;
        }
        let folder = dir.join(folder).join(VERSION);
        zajil_platform::fs::private_dir(&folder).unwrap();
        let name = match tool {
            Tool::YtDlp => "yt-dlp",
            Tool::Ffmpeg => "ffmpeg",
            Tool::Ffprobe => "ffprobe",
            Tool::QuickJs => unreachable!("not among the tools copied here"),
        };
        let mut file = zajil_platform::fs::create_private(&exe(&folder, name)).unwrap();
        file.write_all(&bytes).unwrap();
        zajil_platform::fs::make_executable(&file).unwrap();
        hashes.push((tool, zajil_testserver::sha256_hex(&bytes)));
    }
    Toolbox::with_pins(
        &dir,
        Box::new(move |tool, _| {
            let (_, sha256) = hashes.iter().find(|(t, _)| *t == tool)?;
            Some(Pinned {
                version: VERSION.to_owned(),
                url: "https://example.invalid/tool".to_owned(),
                sha256: sha256.clone(),
                archive: (tool == Tool::YtDlp).then(|| archive.clone()).flatten(),
            })
        }),
    )
}

/// Copies the folder `from` into `into` as an install from a zip leaves it
/// (folders and files only this user can access, `program` executable),
/// and returns each file's path inside it and SHA-256.
fn copy_tree(from: &Path, into: &Path, program: &str) -> Vec<(String, String)> {
    use std::io::Write;
    let mut out = Vec::new();
    let mut folders = vec![(from.to_path_buf(), into.to_path_buf(), String::new())];
    while let Some((src, dst, inside)) = folders.pop() {
        zajil_platform::fs::private_dir(&dst).unwrap();
        for entry in fs::read_dir(&src).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            let relative = if inside.is_empty() {
                name.clone()
            } else {
                format!("{inside}/{name}")
            };
            if entry.file_type().unwrap().is_dir() {
                folders.push((entry.path(), dst.join(&name), relative));
                continue;
            }
            let bytes = fs::read(entry.path()).unwrap();
            let mut file = zajil_platform::fs::create_private(&dst.join(&name)).unwrap();
            file.write_all(&bytes).unwrap();
            if relative == program || cfg!(unix) {
                zajil_platform::fs::make_executable(&file).unwrap();
            }
            out.push((relative, zajil_testserver::sha256_hex(&bytes)));
        }
    }
    out
}

fn exe(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}{}", if cfg!(windows) { ".exe" } else { "" }))
}

/// Runs `ffmpeg-testgen` with `args`, in `dir`.
fn generate(tools: &Tools, dir: &Path, args: &[&str]) {
    let status = Command::new(exe(&tools.testgen, "ffmpeg-testgen"))
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg-testgen {args:?}");
}

/// The media of every test, made once per test in a folder of its own:
/// a 4-second picture (MPEG-4 Part 2, the only LGPL video encoder) with
/// sound (AAC), then the same as fragmented-MP4 HLS, as DASH with separate
/// picture and sound, as Opus sound alone, and two protected HLS playlists.
fn media(tools: &Tools) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    generate(
        tools,
        d,
        &[
            "-filter_complex",
            "testsrc=duration=4:size=320x240:rate=25,format=yuv420p[v];sine=duration=4:frequency=440[a]",
            "-map",
            "[v]",
            "-map",
            "[a]",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
            "clip.mp4",
        ],
    );
    fs::create_dir_all(d.join("hls")).unwrap();
    generate(
        tools,
        d,
        &[
            "-i",
            "clip.mp4",
            "-c",
            "copy",
            "-f",
            "hls",
            "-hls_time",
            "1",
            "-hls_playlist_type",
            "vod",
            "-hls_segment_type",
            "fmp4",
            "hls/index.m3u8",
        ],
    );
    fs::create_dir_all(d.join("dash")).unwrap();
    generate(
        tools,
        d,
        &[
            "-i",
            "clip.mp4",
            "-map",
            "0:v",
            "-map",
            "0:a",
            "-c",
            "copy",
            "-f",
            "dash",
            "dash/manifest.mpd",
        ],
    );
    generate(
        tools,
        d,
        &[
            "-filter_complex",
            "sine=duration=4",
            "-c:a",
            "opus",
            "-strict",
            "experimental",
            "sound.ogg",
        ],
    );
    let playlist = fs::read_to_string(d.join("hls/index.m3u8")).unwrap();
    let with_key = |key: &str| playlist.replacen("#EXT-X-MAP", &format!("{key}\n#EXT-X-MAP"), 1);
    fs::write(
        d.join("hls/fairplay.m3u8"),
        with_key(r#"#EXT-X-KEY:METHOD=SAMPLE-AES,URI="skd://key1",KEYFORMAT="com.apple.streamingkeydelivery",KEYFORMATVERSIONS="1""#),
    )
    .unwrap();
    fs::write(
        d.join("hls/widevine.m3u8"),
        with_key(r#"#EXT-X-KEY:METHOD=SAMPLE-AES-CTR,URI="data:text/plain;base64,AAAA",KEYFORMAT="urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed",KEYFORMATVERSIONS="1""#),
    )
    .unwrap();
    fs::write(
        d.join("page.html"),
        "<!doctype html><html><head><title>صفحة اختبار</title></head><body><video src=\"clip.mp4\" controls></video></body></html>",
    )
    .unwrap();
    dir
}

/// Serves every file under `dir` at its path, with its media type.
fn serve(server: &TestServer, dir: &Path) {
    fn walk(server: &TestServer, root: &Path, dir: &Path) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(server, root, &path);
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let kind = match path.extension().and_then(|e| e.to_str()) {
                Some("m3u8") => "application/vnd.apple.mpegurl",
                Some("mpd") => "application/dash+xml",
                Some("html") => "text/html; charset=utf-8",
                Some("ogg") => "audio/ogg",
                _ => "video/mp4",
            };
            server.put(
                &format!("/{rel}"),
                FileSpec::new(fs::read(&path).unwrap()).content_type(kind),
            );
        }
    }
    walk(server, dir, dir);
}

struct Setup {
    _root: tempfile::TempDir,
    _media: tempfile::TempDir,
    dest: PathBuf,
    runs: PathBuf,
    runner: VideoRunner,
    ffprobe: PathBuf,
}

fn setup(tools: &Tools, server: &TestServer) -> Setup {
    let media = media(tools);
    serve(server, media.path());
    let root = tempfile::tempdir().unwrap();
    let dest = root.path().join("Downloads");
    let runs = root.path().join("run");
    fs::create_dir_all(&dest).unwrap();
    let runner = VideoRunner::new(
        toolbox(tools, root.path()),
        Runs::new(&runs),
        Engine::with_test_root(Options::default(), &test_ca_der()).unwrap(),
    );
    Setup {
        ffprobe: exe(&tools.ffmpeg_dir, "ffprobe"),
        _root: root,
        _media: media,
        dest,
        runs,
        runner,
    }
}

/// The codecs of `file`'s streams and its length in seconds, by ffprobe.
fn probe_file(ffprobe: &Path, file: &Path) -> (Vec<String>, f64) {
    let out = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_name:format=duration",
            "-of",
            "default=nw=1",
        ])
        .arg(file)
        .output()
        .unwrap();
    assert!(out.status.success(), "ffprobe {}", file.display());
    let text = String::from_utf8_lossy(&out.stdout).replace('\r', "");
    let codecs = text
        .lines()
        .filter_map(|l| l.strip_prefix("codec_name="))
        .map(str::to_owned)
        .collect();
    let duration = text
        .lines()
        .find_map(|l| l.strip_prefix("duration="))
        .and_then(|d| d.parse().ok())
        .unwrap_or(0.0);
    (codecs, duration)
}

async fn probe(s: &Setup, url: &str) -> zajil_core::video::Video {
    match s.runner.probe(url, None, &Stop::new()).await.unwrap() {
        Probed::Video(video) => video,
        other => panic!("{other:?}"),
    }
}

async fn download(
    s: &Setup,
    url: &str,
    video: &zajil_core::video::Video,
    choice: Choice,
) -> Result<PathBuf, VideoError> {
    let request = Download {
        url,
        choice: &choice,
        formats: &video.formats,
        cookies: None,
        dest: &s.dest,
        name: &video.title,
    };
    s.runner.download(&request, &Stop::new(), &mut |_| {}).await
}

fn assert_clean(s: &Setup) {
    let left = fs::read_dir(&s.runs).map(|e| e.count()).unwrap_or(0);
    assert_eq!(left, 0, "a run's folder was left");
    let hidden: Vec<String> = fs::read_dir(&s.dest)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(hidden.is_empty(), "{hidden:?}");
}

fn near_4_seconds(duration: f64) {
    assert!((3.9..4.2).contains(&duration), "lasts {duration} s");
}

/// The pinned zip itself (macOS and Windows), installed through Zajil's
/// own code with this build's pins (only its address is the local
/// server's): its SHA-256, then the tree SHA-256 of pins.txt, then a run.
/// A wrong pin would break every install, so it fails here first (review of
/// #278).
#[tokio::test]
async fn the_pinned_zip_installs_through_zajil_s_own_code_and_runs() {
    let Some(tools) = tools() else { return };
    let unpacked = tools.yt_dlp.parent().unwrap();
    if !unpacked.join("_internal").is_dir() {
        return; // Linux: the one-file build.
    }
    let target = zajil_platform::target::current().unwrap();
    let pin = zajil_core::tools::pinned(Tool::YtDlp, target).unwrap();
    let archive = pin.archive.clone().expect("a zip is pinned on this system");
    let zip = unpacked.with_file_name(format!("{}.zip", archive.folder));
    let server = TestServer::https(Tls::trusted(&["localhost", "127.0.0.1"]), "localhost");
    server.put("/yt-dlp.zip", FileSpec::new(fs::read(&zip).unwrap()));
    let root = tempfile::tempdir().unwrap();
    let url = server.url("/yt-dlp.zip");
    let toolbox = Toolbox::with_pins(
        root.path().join("tools"),
        Box::new(move |tool, _| {
            (tool == Tool::YtDlp).then(|| Pinned {
                url: url.clone(),
                ..pin.clone()
            })
        }),
    );
    let engine = Engine::with_test_root(Options::default(), &test_ca_der()).unwrap();
    let path = toolbox
        .install(&engine, Tool::YtDlp, &Stop::new(), &mut |_| {})
        .await
        .unwrap();
    assert!(path.ends_with(&archive.program));
    let checked = toolbox.verified(Tool::YtDlp).await.unwrap();
    let version = Command::new(checked.path())
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        zajil_core::tools::yt_dlp_version()
    );
}

/// The warm-up run (owner's decision, 2026-10-27): yt-dlp starts, loads
/// what it has, and reaches nothing: done well before its time limit, with
/// nothing left behind.
#[tokio::test]
async fn the_warm_up_runs_the_real_yt_dlp_offline_and_leaves_nothing() {
    let Some(tools) = tools() else { return };
    let server = TestServer::http();
    let s = setup(&tools, &server);
    let started = std::time::Instant::now();
    s.runner.warm_up(&Stop::new()).await.unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(120),
        "{:?}",
        started.elapsed()
    );
    assert_clean(&s);
}

#[tokio::test]
async fn a_direct_mp4_downloads_and_plays() {
    let Some(tools) = tools() else { return };
    let server = TestServer::http();
    let s = setup(&tools, &server);
    let url = server.url("/clip.mp4");
    let video = probe(&s, &url).await;
    assert!(!video.formats.is_empty());
    let id = video.formats[0].id.clone();
    let file = download(&s, &url, &video, Choice::Format(id))
        .await
        .unwrap();
    let (codecs, duration) = probe_file(&s.ffprobe, &file);
    assert_eq!(codecs, ["mpeg4", "aac"]);
    near_4_seconds(duration);
    assert_clean(&s);
}

#[tokio::test]
async fn hls_downloads_and_plays() {
    let Some(tools) = tools() else { return };
    let server = TestServer::http();
    let s = setup(&tools, &server);
    let url = server.url("/hls/index.m3u8");
    let video = probe(&s, &url).await;
    let id = video.formats.last().unwrap().id.clone();
    let file = download(&s, &url, &video, Choice::Format(id))
        .await
        .unwrap();
    let (codecs, duration) = probe_file(&s.ffprobe, &file);
    assert_eq!(codecs, ["mpeg4", "aac"]);
    near_4_seconds(duration);
    assert_clean(&s);
}

#[tokio::test]
async fn dash_s_separate_picture_and_sound_are_merged_into_one_file() {
    let Some(tools) = tools() else { return };
    let server = TestServer::http();
    let s = setup(&tools, &server);
    let url = server.url("/dash/manifest.mpd");
    let video = probe(&s, &url).await;
    let picture = video
        .formats
        .iter()
        .find(|f| f.has_video() && !f.has_audio())
        .expect("a picture-only format");
    let sound = video
        .formats
        .iter()
        .find(|f| f.has_audio() && !f.has_video())
        .expect("a sound-only format");
    let choice = Choice::Merge {
        video: picture.id.clone(),
        audio: sound.id.clone(),
    };
    let file = download(&s, &url, &video, choice).await.unwrap();
    let (codecs, duration) = probe_file(&s.ffprobe, &file);
    assert_eq!(codecs, ["mpeg4", "aac"], "one file, both tracks");
    near_4_seconds(duration);
    assert_clean(&s);
}

#[tokio::test]
async fn a_page_with_a_video_tag_downloads_its_video_under_the_page_s_title() {
    let Some(tools) = tools() else { return };
    let server = TestServer::http();
    let s = setup(&tools, &server);
    let url = server.url("/page.html");
    let video = probe(&s, &url).await;
    let id = video.formats[0].id.clone();
    let file = download(&s, &url, &video, Choice::Format(id))
        .await
        .unwrap();
    assert_eq!(probe_file(&s.ffprobe, &file).0, ["mpeg4", "aac"]);
    assert_clean(&s);
}

#[tokio::test]
async fn audio_only_turns_opus_into_m4a_with_our_aac_encoder() {
    let Some(tools) = tools() else { return };
    let server = TestServer::http();
    let s = setup(&tools, &server);
    let url = server.url("/sound.ogg");
    let video = probe(&s, &url).await;
    let file = download(&s, &url, &video, Choice::AudioOnly).await.unwrap();
    assert_eq!(file.extension().unwrap(), "m4a");
    let (codecs, duration) = probe_file(&s.ffprobe, &file);
    assert_eq!(codecs, ["aac"]);
    near_4_seconds(duration);
    assert_clean(&s);
}

#[tokio::test]
async fn protected_hls_downloads_nothing_and_says_why() {
    let Some(tools) = tools() else { return };
    let server = TestServer::http();
    let s = setup(&tools, &server);
    // FairPlay: yt-dlp refuses it itself.
    let err = s
        .runner
        .probe(&server.url("/hls/fairplay.m3u8"), None, &Stop::new())
        .await
        .unwrap_err();
    assert!(matches!(err, VideoError::Drm), "{err}");
    // Widevine: yt-dlp would hand it to ffmpeg; Zajil's check refuses it.
    let url = server.url("/hls/widevine.m3u8");
    let video = probe(&s, &url).await;
    let id = video.formats.last().unwrap().id.clone();
    let err = download(&s, &url, &video, Choice::Format(id))
        .await
        .unwrap_err();
    assert!(matches!(err, VideoError::Drm), "{err}");
    assert!(
        fs::read_dir(&s.dest).unwrap().next().is_none(),
        "nothing written"
    );
    assert_clean(&s);
}

#[tokio::test]
async fn the_link_s_cookies_reach_its_server() {
    let Some(tools) = tools() else { return };
    let server = TestServer::http();
    let s = setup(&tools, &server);
    let url = server.url("/page.html");
    let probed = s
        .runner
        .probe(&url, Some("sid=from-the-browser"), &Stop::new())
        .await
        .unwrap();
    assert!(matches!(probed, Probed::Video(_)));
    let page = server.gets("/page.html");
    assert!(
        page.iter().any(|r| r
            .cookie
            .as_deref()
            .is_some_and(|c| c.contains("sid=from-the-browser"))),
        "{page:?}"
    );
    assert_clean(&s);
}

/// The test authority as PEM, for yt-dlp (which reads PEM files).
fn ca_pem(dir: &Path) -> PathBuf {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let der = test_ca_der();
    let mut b64 = String::new();
    for chunk in der.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            b64.push(if i <= chunk.len() {
                ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
    for line in b64.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(line).unwrap());
        pem.push('\n');
    }
    pem.push_str("-----END CERTIFICATE-----\n");
    let path = dir.join("test-ca.pem");
    fs::write(&path, pem).unwrap();
    path
}

#[tokio::test]
async fn over_https_a_trusted_certificate_works_and_an_untrusted_one_is_refused() {
    let Some(tools) = tools() else { return };
    let server = TestServer::https(Tls::trusted(&["localhost", "127.0.0.1"]), "localhost");
    let s = setup(&tools, &server);
    let url = server.url("/clip.mp4");
    // Without the test authority: refused, with nothing written.
    let err = s.runner.probe(&url, None, &Stop::new()).await.unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("certificate"),
        "{err}"
    );
    // With it (tests only): the same server works. Never by turning the
    // check off.
    let pem_dir = tempfile::tempdir().unwrap();
    let tools_root = tempfile::tempdir().unwrap();
    let runner = VideoRunner::new(
        toolbox(&tools, tools_root.path()),
        Runs::new(&s.runs),
        Engine::with_test_root(Options::default(), &test_ca_der()).unwrap(),
    )
    .with_test_root(ca_pem(pem_dir.path()));
    let probed = runner.probe(&url, None, &Stop::new()).await.unwrap();
    assert!(matches!(probed, Probed::Video(_)));
    assert_clean(&s);
}
