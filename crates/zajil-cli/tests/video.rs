//! `zajil video` (M6): nothing is downloaded without the user's yes, and
//! nothing runs before the tools are there. The downloads themselves are
//! tested in zajil-core (tests/video.rs and tests/video_real.rs).

mod common;

use std::path::Path;
use std::process::{Output, Stdio};

fn zajil(data: &Path, args: &[&str]) -> Output {
    common::zajil_command()
        .arg("--data-dir")
        .arg(data)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn printed(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn the_tools_status_says_what_is_missing() {
    let data = tempfile::tempdir().unwrap();
    let out = zajil(data.path(), &["video", "tools"]);
    assert!(out.status.success(), "{}", printed(&out));
    let text = printed(&out);
    // Every tool is pinned for this system: each one can be downloaded.
    for tool in ["yt-dlp", "ffmpeg", "ffprobe"] {
        assert!(text.contains(&format!("{tool}: not downloaded")), "{text}");
    }
}

#[test]
fn nothing_is_downloaded_without_a_yes() {
    // Not a terminal and no --yes: it explains, asks nothing, downloads
    // nothing (the test has no network to reach anyway).
    let data = tempfile::tempdir().unwrap();
    let out = zajil(data.path(), &["video", "tools", "install"]);
    assert_eq!(out.status.code(), Some(1), "{}", printed(&out));
    let text = printed(&out);
    assert!(text.contains("yt-dlp") && text.contains("LGPL"), "{text}");
    assert!(text.contains("--yes"), "{text}");
    assert!(!data.path().join("tools").join("yt-dlp").exists());
}

#[test]
fn a_video_command_without_the_tools_says_how_to_get_them() {
    let data = tempfile::tempdir().unwrap();
    for args in [
        &["video", "formats", "https://example.com/v"][..],
        &["video", "get", "https://example.com/v"][..],
    ] {
        let out = zajil(data.path(), args);
        assert_eq!(out.status.code(), Some(1), "{}", printed(&out));
        assert!(
            printed(&out).contains("zajil video tools install"),
            "{}",
            printed(&out)
        );
    }
}

#[test]
fn choices_that_contradict_each_other_are_refused_before_anything_runs() {
    let data = tempfile::tempdir().unwrap();
    for args in [
        &["video", "get", "--video", "137", "https://example.com/v"][..],
        &[
            "video",
            "get",
            "--format",
            "18",
            "--audio-only",
            "https://example.com/v",
        ][..],
        &[
            "video",
            "get",
            "--format",
            "18",
            "--video",
            "1",
            "--audio",
            "2",
            "https://example.com/v",
        ][..],
    ] {
        let out = zajil(data.path(), args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", printed(&out));
    }
}

#[test]
fn leftovers_of_a_crash_are_cleaned_by_the_next_video_command() {
    let data = tempfile::tempdir().unwrap();
    let staging = data.path().join("tools").join(".download-0011223344556677");
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("part"), b"x").unwrap();
    let run = data.path().join("run").join("0123456789abcdef");
    zajil_platform::fs::private_dir(&run).unwrap();
    std::fs::write(run.join("cookies.txt"), "secret").unwrap();
    let out = zajil(data.path(), &["video", "tools"]);
    assert!(out.status.success(), "{}", printed(&out));
    assert!(!staging.exists());
    assert!(!run.exists());
}
