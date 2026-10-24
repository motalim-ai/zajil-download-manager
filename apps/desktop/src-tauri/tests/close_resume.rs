//! Closing the app while a download runs, then opening it again: the
//! download goes on from the progress its state file recorded, and the
//! file comes out whole (owner's request on #45). Two ways of closing:
//! asking the app to close (SIGTERM on Linux and macOS, a window close on
//! Windows), and killing it, as if its 5 s had run out or it had crashed.
//!
//! These need the app built by `tauri build`, so `cargo test` lists them as
//! ignored; CI copies the built app aside (as `cargo test` rebuilds the
//! binary without its page) and runs them:
//! `ZAJIL_DESKTOP_APP=<copy> cargo test --test close_resume -- --ignored`
//! (under Xvfb on Linux).

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zajil_core::queue::{Item, QueueStore, Status};
use zajil_core::state::DownloadState;
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

const MIB: u64 = 1024 * 1024;
/// 24 MiB, each connection slowed to 256 KiB/s: about 12 s with 8
/// connections, so the app can be closed in the middle.
const SIZE: u64 = 24 * MIB;

/// The running app, and everything it has written to stderr so far.
struct App {
    child: Child,
    stderr: Arc<Mutex<String>>,
}

impl App {
    fn said(&self, line: &str) -> bool {
        self.stderr.lock().unwrap().contains(line)
    }
}

/// The app never outlives the test, even when the test fails first: killed
/// and waited for (as the CLI tests' `common::Running`, #50). An app that
/// already exited: the kill fails harmlessly and the wait returns at once.
impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The app built by `tauri build`, as CI copies it aside: `cargo test`
/// itself rebuilds the binary without its page (review of #45).
fn start(data: &Path) -> App {
    let app = std::env::var_os("ZAJIL_DESKTOP_APP")
        .expect("ZAJIL_DESKTOP_APP must name the built zajil-desktop");
    let mut child = Command::new(app)
        .arg("--data-dir")
        .arg(data)
        .stderr(Stdio::piped())
        .spawn()
        .expect("the app starts");
    let stderr = Arc::new(Mutex::new(String::new()));
    let (mut pipe, sink) = (child.stderr.take().unwrap(), stderr.clone());
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = pipe.read(&mut buf) {
            if n == 0 {
                break;
            }
            let text = String::from_utf8_lossy(&buf[..n]);
            eprint!("{text}");
            sink.lock().unwrap().push_str(&text);
        }
    });
    let app = App { child, stderr };
    // The page has loaded: a build without its page would otherwise pass
    // (review of #45).
    wait_until("the window is ready", Duration::from_secs(60), || {
        app.said("zajil: the window is ready")
    });
    app
}

/// Asks the app to close and checks that it stopped its downloads and
/// exited cleanly, sooner than its own 5 s limit: a close that only waited
/// the limit out would pass otherwise (review of #45).
fn close_cleanly(app: &mut App, how: Close) {
    let asked = Instant::now();
    ask_to_close(&app.child, how);
    let status = wait_exit(&mut app.child, Duration::from_secs(30));
    let took = asked.elapsed();
    // Let the reader thread take the last lines.
    std::thread::sleep(Duration::from_millis(200));
    assert!(status.success(), "asked to close, it ended with {status:?}");
    assert!(
        app.said("zajil: the downloads stopped; exiting"),
        "it exited without saying that the downloads stopped"
    );
    assert!(took < Duration::from_millis(4500), "closing took {took:?}");
    // Which path stopped the downloads at the end of a Windows session:
    // Zajil's own hook (`zajil_platform::session`), or tao's handling that
    // ends in `RunEvent::Exit` (both stop them the same way). Shown in the
    // CI log (the step runs with --nocapture; review of #59).
    #[cfg(windows)]
    if matches!(how, Close::SessionEnd) {
        let hook = app.said("asked to stop (the Windows session is ending)");
        println!(
            "the Windows session end was handled by {}",
            if hook {
                "zajil_platform::session"
            } else {
                "tao (RunEvent::Exit)"
            }
        );
    }
}

/// How the app is asked to close.
#[derive(Clone, Copy)]
enum Close {
    /// What the system does at a shutdown or when the window is closed:
    /// SIGTERM on Linux and macOS, a window close on Windows.
    Ask,
    /// macOS: the system's quit event (Cmd+Q, the Dock's Quit, a logout).
    #[cfg(target_os = "macos")]
    Quit,
    /// Windows: the session ends (a shutdown, restart or logoff), as the
    /// system says it to the window: `WM_QUERYENDSESSION`, then
    /// `WM_ENDSESSION` (#49). Sent through PowerShell, so the test needs no
    /// unsafe code; a real shutdown cannot be run by a test.
    #[cfg(windows)]
    SessionEnd,
}

fn ask_to_close(child: &Child, how: Close) {
    let pid = child.id().to_string();
    let status = match how {
        #[cfg(unix)]
        Close::Ask => Command::new("kill").args(["-TERM", &pid]).status(),
        #[cfg(windows)]
        Close::Ask => Command::new("taskkill").args(["/PID", &pid]).status(),
        // As Windows does at a real shutdown: both messages to every
        // top-level window of the process (tao's hidden one, the main
        // window...), not only to the one `MainWindowHandle` picks
        // (review of #59).
        #[cfg(windows)]
        Close::SessionEnd => Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "Add-Type -TypeDefinition 'using System; using System.Collections.Generic; using System.Runtime.InteropServices; \
                     public static class ZajilSession {{ \
                       public delegate bool EnumProc(IntPtr h, IntPtr l); \
                       [DllImport(\"user32.dll\")] public static extern bool EnumWindows(EnumProc f, IntPtr l); \
                       [DllImport(\"user32.dll\")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid); \
                       [DllImport(\"user32.dll\")] public static extern IntPtr SendMessageTimeoutW(IntPtr h, uint m, IntPtr w, IntPtr l, uint f, uint t, out IntPtr r); \
                       public static int End(uint pid) {{ \
                         var all = new List<IntPtr>(); \
                         EnumWindows((h, l) => {{ uint p; GetWindowThreadProcessId(h, out p); if (p == pid) all.Add(h); return true; }}, IntPtr.Zero); \
                         IntPtr r; \
                         foreach (var h in all) SendMessageTimeoutW(h, 0x11, IntPtr.Zero, IntPtr.Zero, 2, 5000, out r); \
                         foreach (var h in all) SendMessageTimeoutW(h, 0x16, (IntPtr)1, IntPtr.Zero, 2, 10000, out r); \
                         return all.Count; }} }}'; \
                     $n = [ZajilSession]::End({pid}); Write-Output \"session end sent to $n top-level windows\"; if ($n -eq 0) {{ exit 1 }}"
                ),
            ])
            .status(),
        #[cfg(target_os = "macos")]
        Close::Quit => Command::new("osascript")
            .args([
                "-l",
                "JavaScript",
                "-e",
                &format!(
                    "ObjC.import('AppKit'); $.NSRunningApplication.runningApplicationWithProcessIdentifier({pid}).terminate"
                ),
            ])
            .status(),
    };
    assert!(status.expect("the close request is sent").success());
}

fn wait_exit(child: &mut Child, within: Duration) -> ExitStatus {
    let deadline = Instant::now() + within;
    loop {
        if let Some(status) = child.try_wait().expect("the app's status") {
            return status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the app did not exit within {within:?}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The one download, read from the files as another process would.
fn item(store: &QueueStore) -> Item {
    let queues = store.load().expect("the queues file");
    queues
        .queues
        .iter()
        .flat_map(|q| q.items.clone())
        .next()
        .expect("one download")
}

/// The bytes its state file says are on disk.
fn recorded(item: &Item) -> u64 {
    item.state_file
        .as_deref()
        .and_then(|p| DownloadState::load(p).ok())
        .map_or(0, |s| s.bytes_done())
}

fn wait_until(what: &str, within: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + within;
    while !done() {
        assert!(Instant::now() < deadline, "{what}: not within {within:?}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// `None`: killed; otherwise closed that way.
fn close_in_the_middle_then_open_again(close: Option<Close>) {
    let kill = close.is_none();
    let server = TestServer::http();
    let body = random_bytes(SIZE as usize, if kill { 46 } else { 45 });
    server.put("/big.bin", FileSpec::new(body.clone()).throttle(256 * 1024));
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data.path());
    store
        .update(|q| q.add("main", &server.url("/big.bin"), downloads.path()))
        .unwrap();

    let mut app = start(data.path());
    wait_until("2 MiB recorded", Duration::from_secs(90), || {
        recorded(&item(&store)) >= 2 * MIB
    });
    if kill {
        app.child.kill().expect("the app is killed");
        wait_exit(&mut app.child, Duration::from_secs(30));
    } else if let Some(how) = close {
        close_cleanly(&mut app, how);
    }
    let before = item(&store);
    let progress = recorded(&before);
    assert_ne!(before.status, Status::Done);
    assert!(
        (2 * MIB..SIZE).contains(&progress),
        "recorded {progress} of {SIZE}"
    );
    let requests_before = server.gets("/big.bin").len();

    // Opened again, it goes on without being asked.
    let mut app = start(data.path());
    wait_until("the download finished", Duration::from_secs(120), || {
        item(&store).status == Status::Done
    });
    close_cleanly(&mut app, Close::Ask);

    let done = item(&store);
    let file = done.file.as_deref().expect("the finished file");
    assert_eq!(
        sha256_hex(&std::fs::read(file).unwrap()),
        sha256_hex(&body),
        "the file is whole"
    );
    // From the recorded progress, not from the start: every request after
    // opening again asked for a range, and together they sent no more than
    // what was missing (a margin of 1 MiB for a part re-asked at a split).
    let after: Vec<_> = server.gets("/big.bin").split_off(requests_before);
    assert!(!after.is_empty());
    assert!(after.iter().all(|r| r.range.is_some()), "{after:?}");
    let sent: u64 = after.iter().map(|r| r.bytes_sent).sum();
    assert!(
        sent <= SIZE - progress + MIB,
        "sent {sent} after opening again; {} were missing",
        SIZE - progress
    );
}

#[test]
#[ignore = "needs the built app (ZAJIL_DESKTOP_APP); run by CI after the build"]
fn closed_during_a_download_it_goes_on_when_opened_again() {
    close_in_the_middle_then_open_again(Some(Close::Ask));
}

/// macOS: Cmd+Q, the Dock's Quit and a logout send the system's quit event,
/// which ends the program without asking it (review of #45).
#[cfg(target_os = "macos")]
#[test]
#[ignore = "needs the built app (ZAJIL_DESKTOP_APP); run by CI after the build"]
fn quit_during_a_download_it_goes_on_when_opened_again() {
    close_in_the_middle_then_open_again(Some(Close::Quit));
}

/// Windows: a shutdown, restart or logoff during a download (#49).
#[cfg(windows)]
#[test]
#[ignore = "needs the built app (ZAJIL_DESKTOP_APP); run by CI after the build"]
fn session_end_during_a_download_it_goes_on_when_opened_again() {
    close_in_the_middle_then_open_again(Some(Close::SessionEnd));
}

#[test]
#[ignore = "needs the built app (ZAJIL_DESKTOP_APP); run by CI after the build"]
fn killed_during_a_download_it_goes_on_when_opened_again() {
    close_in_the_middle_then_open_again(None);
}
