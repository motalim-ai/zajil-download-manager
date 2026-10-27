//! A stand-in for yt-dlp in the tests of `zajil_core::video` (built only
//! with the `test-support` feature, never shipped).
//!
//! It never reads its environment for instructions: the runner clears it.
//! What to do comes from the file `script` in the folder it gets with
//! `--ffmpeg-location`, which the test chooses; everything it was given (its
//! arguments, environment and working folder, and the cookie file's text
//! and permissions) is written to `calls.jsonl` there.

use std::io::Write;
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value_of = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let control = PathBuf::from(value_of("--ffmpeg-location").expect("--ffmpeg-location"));
    let cookies = value_of("--cookies").map(PathBuf::from);
    record(&control, &args, cookies.as_deref());
    let script = std::fs::read_to_string(control.join("script")).unwrap_or_default();
    let mut words = script.split_whitespace();
    match words.next().unwrap_or("") {
        // Prints probe.json as the probe's output.
        "probe" => {
            let json = std::fs::read(control.join("probe.json")).unwrap();
            std::io::stdout().write_all(&json).unwrap();
        }
        // Writes media.<ext> in its folder with progress lines, and prints
        // its path, like --print after_move:filepath.
        "download" => {
            let ext = words.next().unwrap_or("mp4");
            let file = std::env::current_dir()
                .unwrap()
                .join(format!("media.{ext}"));
            let data = vec![7u8; 1000];
            std::fs::write(&file, &data).unwrap();
            println!("zajil-progress 500 1000 NA 250.0");
            println!("zajil-progress 1000 1000 NA NA");
            println!("{}", file.display());
        }
        // Signed in to YouTube with no JavaScript runtime: with a cookie
        // file only pictures are left and it fails as yt-dlp does; without
        // one it prints probe.json.
        "signed-in" => {
            if cookies.is_some() {
                eprintln!("WARNING: [youtube] abc: n challenge solving failed");
                eprintln!(
                    "ERROR: [youtube] abc: Requested format is not available. \
                     Use --list-formats for a list of available formats"
                );
                std::process::exit(1);
            }
            let json = std::fs::read(control.join("probe.json")).unwrap();
            std::io::stdout().write_all(&json).unwrap();
        }
        // Like `download`, in two parts as a merge goes: the picture
        // (1000 bytes), then the sound (200), each counted from 0.
        "download-parts" => {
            let file = std::env::current_dir().unwrap().join("media.mp4");
            std::fs::write(&file, vec![7u8; 1200]).unwrap();
            println!("zajil-progress 500 1000 NA 250.0");
            println!("zajil-progress 1000 1000 NA NA");
            println!("zajil-progress 100 200 NA NA");
            println!("zajil-progress 200 200 NA NA");
            println!("{}", file.display());
        }
        // A page it cannot read (its link has `/page`), and a video's own
        // link it can: prints probe.json for that one.
        "page-unsupported" => {
            let url = args.last().unwrap();
            if url.contains("/page") {
                eprintln!("ERROR: Unsupported URL: {url}");
                std::process::exit(1);
            }
            let json = std::fs::read(control.join("probe.json")).unwrap();
            std::io::stdout().write_all(&json).unwrap();
        }
        // Prints a path outside its folder.
        "escape" => {
            let outside = control.join("outside.mp4");
            std::fs::write(&outside, b"x").unwrap();
            println!("{}", outside.display());
        }
        // Fails with the rest of the script as its message, on stderr.
        "fail" => {
            let message: Vec<&str> = words.collect();
            eprintln!("ERROR: {}", message.join(" "));
            std::process::exit(1);
        }
        // The warm-up as yt-dlp runs it with no network: its verbose
        // header, then the request fails.
        "warm" => {
            eprintln!("[debug] Optional libraries: certifi, requests");
            eprintln!("[debug] Request Handlers: urllib, requests");
            eprintln!(
                "ERROR: [youtube] abc: Unable to download API page: Unable to connect to proxy"
            );
            std::process::exit(1);
        }
        // Says it started, then waits to be ended (a stop, a crash).
        "hang" => {
            std::fs::write(control.join("started"), b"").unwrap();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        }
        other => panic!("unknown script {other:?}"),
    }
}

fn record(control: &Path, args: &[String], cookies: Option<&Path>) {
    let env: Vec<(String, String)> = std::env::vars().collect();
    let (cookie_text, cookie_private) = match cookies {
        Some(path) => (std::fs::read_to_string(path).ok(), private(path)),
        None => (None, None),
    };
    let line = serde_json::json!({
        "args": args,
        "env": env,
        "cwd": std::env::current_dir().unwrap(),
        "cookie_text": cookie_text,
        "cookie_private": cookie_private,
    });
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(control.join("calls.jsonl"))
        .unwrap();
    writeln!(file, "{line}").unwrap();
}

/// Read from the file system itself, on every system.
fn private(path: &Path) -> Option<bool> {
    zajil_platform::fs::only_owner_can_access(path).ok()
}
