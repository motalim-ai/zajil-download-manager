//! M5.5's end-to-end test, with the test server: a link through «Add
//! link», «Download file info» and «Start download», its progress window,
//! «Download complete», and «Open folder». The real program and its real
//! windows: each clicks its own button, found by the label the user reads
//! (`src/uitest/e2e.ts`), and says its step (`ui_test.rs`); «Open folder»
//! is written down instead of opening a file manager. And the same from
//! the browser (the owner's notes on 1.0): a download the extension sends
//! opens «Download file info» by itself.
//!
//! It needs the app built with the `ui-test` feature, so `cargo test`
//! lists it as ignored; CI copies that build aside and runs it:
//! `ZAJIL_DESKTOP_APP=<copy> cargo test --test end_to_end -- --ignored`
//! (under Xvfb on Linux).

use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use zajil_core::queue::{QueueStore, Status};
use zajil_core::settings::SettingsStore;
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

const MIB: u64 = 1024 * 1024;
/// 8 MiB, each connection slowed to 256 KiB/s: a few seconds, so the
/// progress window shows while it runs.
const SIZE: u64 = 8 * MIB;

/// One app at a time: the two tests share the screen, and CI's folder of
/// steps.
static ONE_APP: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The app never outlives the test (as in `close_resume.rs`).
struct App(Child);

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Step `name` as the app wrote it, waited for `within`; a window that
/// could not do its part says so in `e2e-failed.json`.
fn step(out: &Path, app: &mut App, name: &str, within: Duration) -> serde_json::Value {
    let deadline = Instant::now() + within;
    loop {
        if let Ok(bytes) = std::fs::read(out.join(format!("e2e-{name}.json"))) {
            let step: serde_json::Value = serde_json::from_slice(&bytes).expect("a step's report");
            println!("step {name}: {step}");
            return step;
        }
        if let Ok(failed) = std::fs::read_to_string(out.join("e2e-failed.json")) {
            panic!("waiting for step {name}, a window could not go on: {failed}");
        }
        if let Some(status) = app.0.try_wait().expect("the app's status") {
            panic!("waiting for step {name}, the app ended: {status:?}");
        }
        if Instant::now() >= deadline {
            screen(out);
            panic!("step {name}: not within {within:?}");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Windows: a picture of the whole screen, next to the steps (#220: the
/// page did not load there, and nothing said why).
fn screen(out: &Path) {
    if !cfg!(windows) {
        return;
    }
    let file = out.join("e2e-screen.png");
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms, System.Drawing; \
         $s = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds; \
         $b = New-Object System.Drawing.Bitmap $s.Width, $s.Height; \
         [System.Drawing.Graphics]::FromImage($b).CopyFromScreen($s.Location, [System.Drawing.Point]::Empty, $s.Size); \
         $b.Save('{}', [System.Drawing.Imaging.ImageFormat]::Png)",
        file.display().to_string().replace('\'', "''")
    );
    let saved = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .status();
    println!("the screen: {} ({saved:?})", file.display());
}

/// The app, started on a data folder of its own with `e2e` (a link, or
/// `browser`), saving into a download folder of its own; the steps go to
/// `<ZAJIL_E2E_OUT or a folder of its own>/<name>`.
struct Run {
    app: App,
    out: std::path::PathBuf,
    data: tempfile::TempDir,
    downloads: tempfile::TempDir,
    _own: tempfile::TempDir,
}

fn start(name: &str, e2e: &str) -> Run {
    let (data, downloads) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    // The steps (and a picture of the screen if one does not come): in
    // ZAJIL_E2E_OUT if set (CI uploads it), otherwise a folder of its own.
    let own = tempfile::tempdir().unwrap();
    let out = std::env::var_os("ZAJIL_E2E_OUT")
        .map_or_else(|| own.path().to_path_buf(), std::path::PathBuf::from)
        .join(name);
    std::fs::create_dir_all(&out).unwrap();
    // The download folder: never the CI machine's own.
    SettingsStore::new(data.path())
        .update(|s| {
            s.download_dir = Some(downloads.path().to_path_buf());
            Ok(())
        })
        .unwrap();
    let program = std::env::var_os("ZAJIL_DESKTOP_APP")
        .expect("ZAJIL_DESKTOP_APP must name the zajil-desktop built with ui-test");
    let app = App(Command::new(program)
        .arg("--data-dir")
        .arg(data.path())
        .arg("--ui-test-e2e")
        .arg(e2e)
        .arg("--ui-test-out")
        .arg(&out)
        .spawn()
        .expect("the app starts"));
    Run {
        app,
        out,
        data,
        downloads,
        _own: own,
    }
}

/// The one download of `data`, once done: whole, under `downloads`, as
/// «Open folder» (`open`) showed it.
fn check_done(
    data: &Path,
    downloads: &Path,
    body: &[u8],
    open: &serde_json::Value,
) -> zajil_core::queue::Item {
    let queues = QueueStore::new(data).load().expect("the queues file");
    let item = queues
        .queues
        .iter()
        .flat_map(|q| q.items.iter())
        .next()
        .expect("one download")
        .clone();
    assert_eq!(item.status, Status::Done);
    let file = item.file.clone().expect("the finished file");
    // Under it (a category may have its own folder there).
    assert!(
        file.canonicalize()
            .unwrap()
            .starts_with(downloads.canonicalize().unwrap()),
        "{} is not under {}",
        file.display(),
        downloads.display()
    );
    assert_eq!(
        sha256_hex(&std::fs::read(&file).unwrap()),
        sha256_hex(body),
        "the file is whole"
    );
    assert_eq!(open["detail"]["folder"], true);
    assert_eq!(open["detail"]["path"], file.to_str().unwrap());
    item
}

#[test]
#[ignore = "needs the app built with ui-test (ZAJIL_DESKTOP_APP); run by CI after that build"]
fn a_link_goes_from_add_link_to_open_folder() {
    let _one = ONE_APP.lock().unwrap_or_else(|e| e.into_inner());
    let server = TestServer::http();
    let body = random_bytes(SIZE as usize, 55);
    server.put(
        "/lecture.bin",
        FileSpec::new(body.clone()).throttle(256 * 1024),
    );
    let link = server.url("/lecture.bin");
    let Run {
        mut app,
        out,
        data,
        downloads,
        ..
    } = start("link", &link);

    let minute = Duration::from_secs(60);
    step(&out, &mut app, "start", 2 * minute);
    let link_step = step(&out, &mut app, "link", minute);
    assert_eq!(link_step["detail"]["url"], link.as_str());
    let info = step(&out, &mut app, "info", minute);
    assert_eq!(info["detail"]["name"], "lecture.bin");
    let progress = step(&out, &mut app, "progress", minute);
    assert_eq!(progress["detail"]["name"], "lecture.bin");
    let complete = step(&out, &mut app, "complete", 2 * minute);
    assert_eq!(complete["detail"]["name"], "lecture.bin");
    let open = step(&out, &mut app, "open", minute);
    // In that order, each once.
    let order: Vec<_> = [&link_step, &info, &progress, &complete, &open]
        .iter()
        .map(|s| s["n"].as_u64().unwrap())
        .collect();
    assert_eq!(order, [2, 3, 4, 5, 6]);
    check_done(data.path(), downloads.path(), &body, &open);
}

/// A program from the browser: «Download file info» opens by itself, and
/// its «Start download» is the user's OK, so the download is never held
/// for it (the owner's notes on 1.0).
#[test]
#[ignore = "needs the app built with ui-test (ZAJIL_DESKTOP_APP); run by CI after that build"]
fn a_download_from_the_browser_opens_its_own_info_window() {
    let _one = ONE_APP.lock().unwrap_or_else(|e| e.into_inner());
    let server = TestServer::http();
    let body = random_bytes(SIZE as usize, 56);
    server.put(
        "/setup.exe",
        FileSpec::new(body.clone()).throttle(256 * 1024),
    );
    let link = server.url("/setup.exe");
    let Run {
        mut app,
        out,
        data,
        downloads,
        ..
    } = start("browser", "browser");

    let minute = Duration::from_secs(60);
    step(&out, &mut app, "start", 2 * minute);
    // As the extension sends it, through `zajil-host`'s channel.
    let request = serde_json::json!({
        "type": "download",
        "protocol_version": zajil_host::PROTOCOL_VERSION,
        "url": link,
        "referrer": server.url("/downloads.html"),
        "filename": "setup.exe",
    });
    let mut channel = zajil_platform::ipc::connect(data.path(), Duration::from_secs(30))
        .expect("the app's channel");
    zajil_host::write_json(&mut channel, &request).unwrap();
    let answer = zajil_host::read_frame(&mut channel, zajil_host::MAX_RESPONSE)
        .unwrap()
        .expect("an answer");
    let answer: zajil_host::Response = serde_json::from_slice(&answer).unwrap();
    assert!(answer.ok, "the app took it: {answer:?}");

    let info = step(&out, &mut app, "info", minute);
    assert_eq!(info["detail"]["name"], "setup.exe");
    let progress = step(&out, &mut app, "progress", minute);
    assert_eq!(progress["detail"]["name"], "setup.exe");
    let complete = step(&out, &mut app, "complete", 2 * minute);
    assert_eq!(complete["detail"]["name"], "setup.exe");
    let open = step(&out, &mut app, "open", minute);
    let order: Vec<_> = [&info, &progress, &complete, &open]
        .iter()
        .map(|s| s["n"].as_u64().unwrap())
        .collect();
    assert_eq!(order, [2, 3, 4, 5]);
    let item = check_done(data.path(), downloads.path(), &body, &open);
    assert!(item.ok_given && !item.needs_ok, "never held: {item:?}");
}
