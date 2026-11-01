//! The test server for the app on the emulator (android/README.md): the
//! local `zajil-testserver`, never the internet.
//!
//!   cargo run -p zajil-android --example emulator_server -- <log file>
//!   adb reverse tcp:8099 tcp:8099
//!
//! The app reaches it at 127.0.0.1 on the device, through `adb reverse`, not
//! at the emulator's 10.0.2.2: from Android 17 an app needs the "local
//! network" permission for a private address, and Zajil does not ask for it.
//!
//! - `http://127.0.0.1:8099/<random folder>/file.bin` (printed at start): 50
//!   MB of random bytes, each connection slowed to 256 KiB/s, so that a
//!   pause or a kill lands in the middle. And `small.bin`, 8 MB, for the
//!   queue.
//! - An HTTPS port with a self-signed certificate, printed at start with its
//!   own `adb reverse` line: the app must refuse it.
//!
//! With ZAJIL_SERVE_DIR, every file under that folder too, at its path
//! (the update test, A4; percent-encoded, and under `slow/` at 256 KiB/s
//! per connection, for the screens), and the base address is printed.
//!
//! Every request (its range and the bytes sent) is written to the log file
//! every second, to check that a resume asks only for what is missing.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};
use zajil_testserver::{FileSpec, TestServer};

fn main() {
    let log = std::env::args()
        .nth(1)
        .expect("usage: emulator_server <log file>");
    let mut body = vec![0u8; 50 * 1024 * 1024];
    getrandom::fill(&mut body).unwrap();
    let sha = Sha256::digest(&body);

    let server = TestServer::http_on(([127, 0, 0, 1], 8099).into()).expect("port 8099");
    server.put("/file.bin", FileSpec::new(body).throttle(256 * 1024));
    // Smaller, for the queue: several of them one after another.
    let mut small = vec![0u8; 8 * 1024 * 1024];
    getrandom::fill(&mut small).unwrap();
    server.put("/small.bin", FileSpec::new(small).throttle(256 * 1024));
    let tls = TestServer::https_self_signed(&["127.0.0.1"], "127.0.0.1");
    tls.put("/file.bin", FileSpec::new(vec![7u8; 1024]));

    println!("sha256 {}", hex(&sha));
    // The server puts a random folder in front of every path.
    println!("http  {}", server.url("/file.bin"));
    println!("https {}", tls.url("/file.bin"));
    println!("adb reverse tcp:{0} tcp:{0}", tls.addr().port());
    // The update test (A4): every file under ZAJIL_SERVE_DIR, at its path,
    // served again when it changes (to swap a good APK for a changed one).
    let serve = std::env::var_os("ZAJIL_SERVE_DIR").map(PathBuf::from);
    let mut served = HashMap::new();
    if let Some(dir) = &serve {
        serve_dir(&server, dir, dir, &mut served);
        println!("base  {}", server.url("").trim_end_matches('/'));
    }
    loop {
        std::thread::sleep(Duration::from_secs(1));
        if let Some(dir) = &serve {
            serve_dir(&server, dir, dir, &mut served);
        }
        let mut out = std::fs::File::create(&log).unwrap();
        for request in server.requests() {
            writeln!(
                out,
                "{} {} range={} sent={} status={} cookie={} referer={}",
                request.method,
                request.path,
                request.range.as_deref().unwrap_or("-"),
                request.bytes_sent,
                request.status,
                request.cookie.as_deref().unwrap_or("-"),
                request.referer.as_deref().unwrap_or("-"),
            )
            .unwrap();
        }
    }
}

/// Serves the files under `dir` that are new or changed since `served`.
fn serve_dir(
    server: &TestServer,
    root: &Path,
    dir: &Path,
    served: &mut HashMap<PathBuf, SystemTime>,
) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            serve_dir(server, root, &path, served);
            continue;
        }
        let changed = std::fs::metadata(&path).and_then(|m| m.modified()).unwrap();
        if served.get(&path) == Some(&changed) {
            continue;
        }
        served.insert(path.clone(), changed);
        let at = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        // A file under «slow/» comes at 256 KiB/s per connection, as
        // file.bin, to see a download run; a name in Arabic is asked for
        // percent-encoded.
        let mut spec = FileSpec::new(std::fs::read(&path).unwrap());
        // Its type by its extension: pages for the in-app browser (A6).
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        let kind = match ext {
            "html" => Some("text/html; charset=utf-8"),
            "m3u8" => Some("application/vnd.apple.mpegurl"),
            "mp4" => Some("video/mp4"),
            "ts" => Some("video/mp2t"),
            _ => None,
        };
        if let Some(kind) = kind {
            spec = spec.content_type(kind);
        }
        let spec = if at.starts_with("slow/") {
            spec.throttle(256 * 1024)
        } else {
            spec
        };
        server.put(&format!("/{}", encoded(&at)), spec);
    }
}

/// `path` as a link has it: bytes other than letters, digits and `-._~/`
/// percent-encoded.
fn encoded(path: &str) -> String {
    path.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
