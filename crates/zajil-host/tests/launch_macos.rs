//! macOS: the host starts Zajil through LaunchServices, not as its own
//! child. Chrome marks its own process (`com.apple.quarantine`), and the
//! system marks every program it starts the same way, and every file they
//! write: Zajil's queues, and yt-dlp's Python.framework, which Gatekeeper
//! then refused as "damaged" (owner's report before 1.0).
//!
//! This test stands in for the browser, marked as Chrome marks itself; a
//! `Zajil.app` of its own, with the real `zajil-host` and a script for the
//! app, stands in for Zajil.
#![cfg(target_os = "macos")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use zajil_host::{MAX_REQUEST, Request, Response, read_frame, write_frame};
use zajil_platform::origin;

/// The host, killed and waited for even when the test fails first.
struct Host(Child);

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The app's stand-in on the channel, once the started app said it
/// started: every request that reaches it is sent on.
fn serve(data_dir: &Path) -> mpsc::Receiver<Request> {
    let listener = zajil_platform::ipc::Listener::bind(data_dir).expect("the channel opens");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(mut stream) = listener.accept() {
            let Ok(Some(message)) = read_frame(&mut stream, MAX_REQUEST) else {
                continue;
            };
            let _ = tx.send(serde_json::from_slice(&message).expect("a request"));
            let _ = write_frame(&mut stream, &serde_json::to_vec(&Response::ok()).unwrap());
        }
    });
    rx
}

/// A `Zajil.app` in `dir`: the real host, and for the app a script that
/// writes, as Zajil writes its queues, a folder and a file holding the
/// arguments it was started with.
fn zajil_app(dir: &Path) -> std::path::PathBuf {
    let bundle = dir.join("Zajil.app");
    let programs = bundle.join("Contents").join("MacOS");
    fs::create_dir_all(&programs).unwrap();
    fs::write(
        bundle.join("Contents").join("Info.plist"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>zajil-desktop</string>
<key>CFBundleIdentifier</key><string>app.zajil.test.launch{}</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
"#,
            std::process::id()
        ),
    )
    .unwrap();
    let app = programs.join("zajil-desktop");
    fs::write(
        &app,
        "#!/bin/sh\n\
         [ \"$1\" = --data-dir ] && [ $# -eq 2 ] || exit 2\n\
         mkdir \"$2/written\" || exit 3\n\
         printf '%s\\n' \"$@\" > \"$2/written/args.new\" && mv \"$2/written/args.new\" \"$2/written/args\"\n",
    )
    .unwrap();
    fs::set_permissions(&app, fs::Permissions::from_mode(0o755)).unwrap();
    let host = programs.join("zajil-host");
    fs::copy(env!("CARGO_BIN_EXE_zajil-host"), &host).unwrap();
    host
}

#[test]
fn a_zajil_the_marked_browser_starts_is_not_marked() {
    let place = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    // Made before this process is marked: a marked app would not start.
    let host = zajil_app(place.path());

    origin::quarantine_this_process("com.google.Chrome").unwrap();
    let probe = place.path().join("probe");
    fs::write(&probe, b"").unwrap();
    assert!(
        origin::read(&probe).unwrap().is_some(),
        "this process should be marked now"
    );

    let mut host = Host(
        Command::new(&host)
            .arg("chrome-extension://abcdefghijklmnopabcdefghijklmnop/")
            .env("ZAJIL_DATA_DIR", data.path())
            .env_remove("ZAJIL_NO_LAUNCH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("zajil-host starts"),
    );
    // A host started by the marked browser is marked too, as Chrome's are.
    write_frame(
        host.0.stdin.as_mut().unwrap(),
        br#"{"type":"download","protocol_version":1,"url":"https://example.com/a.zip"}"#,
    )
    .unwrap();

    let args = data.path().join("written").join("args");
    let deadline = Instant::now() + Duration::from_secs(15);
    while !args.exists() {
        assert!(Instant::now() < deadline, "the app was not started");
        std::thread::sleep(Duration::from_millis(50));
    }
    let arrived = serve(data.path());
    let answer = read_frame(host.0.stdout.as_mut().unwrap(), 1 << 20)
        .unwrap()
        .expect("an answer");
    assert_eq!(
        serde_json::from_slice::<Response>(&answer).unwrap(),
        Response::ok()
    );
    assert!(matches!(
        arrived.recv_timeout(Duration::from_secs(10)).unwrap(),
        Request::Download { .. }
    ));

    assert_eq!(
        fs::read_to_string(&args).unwrap(),
        format!("--data-dir\n{}\n", data.path().display())
    );
    for written in [args.parent().unwrap(), &args] {
        assert_eq!(
            origin::read(written).unwrap(),
            None,
            "{}",
            written.display()
        );
    }
}
