//! Every window a button opens really loads its page, not only its frame.
//! On Windows a window made by a command that is not async waited for the
//! main thread forever: its frame came, its page never did, and the main
//! window stopped answering (#226's end-to-end test, Tauri's documented
//! deadlock). The main window opens them all through the page's own calls
//! (`src/uitest/windows.ts`), and each window says its page ran.
//!
//! It needs the app built with the `ui-test` feature, so `cargo test`
//! lists it as ignored; CI copies that build aside and runs it:
//! `ZAJIL_DESKTOP_APP=<copy> cargo test --test windows_open -- --ignored`
//! (under Xvfb on Linux).

use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// The windows the main window opens, by label: «Add link», «Download
/// file info», a progress window by its button and one after «Start
/// download», the schedule, «Batch download», «Site grabber», the video
/// window and the drop box.
const WINDOWS: [&str; 9] = [
    "add-link",
    "add-info",
    "progress-1",
    "progress-2",
    "schedule",
    "batch",
    "grab",
    "video",
    "drop",
];

/// The app never outlives the test (as in `close_resume.rs`).
struct App(Child);

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "needs the app built with ui-test (ZAJIL_DESKTOP_APP); run by CI after that build"]
fn every_window_a_button_opens_loads_its_page() {
    let (data, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let program = std::env::var_os("ZAJIL_DESKTOP_APP")
        .expect("ZAJIL_DESKTOP_APP must name the zajil-desktop built with ui-test");
    let mut app = App(Command::new(program)
        .arg("--data-dir")
        .arg(data.path())
        .arg("--ui-test-windows")
        .arg("--ui-test-out")
        .arg(out.path())
        .spawn()
        .expect("the app starts"));
    let loaded = |label: &str| out.path().join(format!("loaded-{label}.json")).exists();
    // The main window's page first, then the windows one after the other.
    let deadline = Instant::now() + Duration::from_secs(120);
    while !WINDOWS.iter().all(|w| loaded(w)) && Instant::now() < deadline {
        if let Some(status) = app.0.try_wait().expect("the app's status") {
            panic!("the app ended: {status:?}");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let mut refused: Vec<String> = std::fs::read_dir(out.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("loaded-refused-"))
        .map(|n| std::fs::read_to_string(out.path().join(n)).unwrap())
        .collect();
    refused.sort();
    let missing: Vec<_> = WINDOWS.iter().filter(|w| !loaded(w)).collect();
    println!(
        "loaded: {:?}",
        WINDOWS.iter().filter(|w| loaded(w)).collect::<Vec<_>>()
    );
    assert!(refused.is_empty(), "opening was refused: {refused:?}");
    assert!(
        missing.is_empty(),
        "these windows' pages never ran within 120 s: {missing:?}"
    );
}
