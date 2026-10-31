//! How fast the windows open, for the developers: with `ZAJIL_MEASURE=1`,
//! Zajil opens and closes «Add a link», «Download file info» and a
//! progress window a few times by itself once its main window is up, and
//! prints how long each took from the open to its page's first frame
//! (`zajil-measure: …` lines). `ZAJIL_POOL=0` opens them without the
//! windows made beforehand, as before them. Use it with `--data-dir` on a
//! folder of its own; nothing is downloaded.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// When Zajil started, for the main window's own time.
static STARTED: LazyLock<Instant> = LazyLock::new(Instant::now);

/// When each window was asked to open, by role.
static ASKED: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);

pub fn on() -> bool {
    std::env::var_os("ZAJIL_MEASURE").is_some_and(|v| v == "1")
}

/// `ZAJIL_MEASURE=<window>`: that window opens at the start, to look at
/// (`settings`, `add-info`), and nothing is timed.
pub fn show() -> Option<String> {
    std::env::var("ZAJIL_MEASURE")
        .ok()
        .filter(|v| v == "settings" || v == "add-info")
}

/// Whether the windows are made beforehand (`ZAJIL_POOL=0`: no).
pub fn pool_on() -> bool {
    std::env::var_os("ZAJIL_POOL").is_none_or(|v| v != "0")
}

/// Zajil starts: its clock starts.
pub fn start() {
    LazyLock::force(&STARTED);
}

/// Window `role` is asked to open now.
pub fn asked(role: &str) {
    if on() {
        let now = Instant::now();
        ASKED.lock().unwrap().insert(role.to_owned(), now);
        SHOWN.lock().unwrap().insert(role.to_owned(), now);
    }
}

/// The page of the window that asks drew its first frame.
#[tauri::command]
pub fn window_painted<R: tauri::Runtime>(window: tauri::WebviewWindow<R>) {
    if !on() {
        return;
    }
    let role = crate::windows::role_of(&window);
    let took = match ASKED.lock().unwrap().remove(&role) {
        Some(at) => at.elapsed(),
        None if role == "main" => STARTED.elapsed(),
        None => return,
    };
    eprintln!(
        "zajil-measure: {role} {:.0} ms",
        took.as_secs_f64() * 1000.0
    );
}

/// A window that took its role shows now ([`crate::windows::window_ready`]).
pub fn shown<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    if !on() {
        return;
    }
    let role = crate::windows::role_of(window);
    if let Some(at) = SHOWN.lock().unwrap().remove(&role) {
        eprintln!(
            "zajil-measure: {role} shown after {:.0} ms",
            at.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// When each window was asked to open, by role, for [`shown`].
static SHOWN: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);

/// A window grew or shrank to what its page shows, after it opened.
pub fn resized<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>, from: f64, to: f64) {
    if on() {
        eprintln!(
            "zajil-measure: {} resized from {from:.0} to {to:.0} after it opened ({}, at {:.0} ms)",
            crate::windows::role_of(window),
            if window.is_visible().unwrap_or(false) {
                "shown"
            } else {
                "hidden"
            },
            STARTED.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// Opens and closes each kind of window `ROUNDS` times, once the spare
/// windows had time to load.
pub fn drive<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    const ROUNDS: usize = 6;
    let handle = handle.clone();
    if let Some(window) = show() {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let size = if window == crate::windows::SETTINGS {
                crate::windows::SETTINGS_SIZE
            } else {
                use tauri::Manager;
                handle
                    .state::<crate::windows::Shared>()
                    .lock()
                    .unwrap()
                    .set_info(
                        &window,
                        crate::windows::InfoStart {
                            url: std::env::var("ZAJIL_MEASURE_URL").unwrap_or_default(),
                            offer: None,
                        },
                    );
                crate::windows::INFO_SIZE
            };
            let _ = crate::windows::open(&handle, &window, "Zajil", size);
            if let Some(window) = crate::windows::find(&handle, &window) {
                let _ = window.set_always_on_top(true);
            }
        });
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        // The list as the main window reads it, four times a second while
        // downloads run: the first read, then the next ones.
        {
            use tauri::Manager;
            let app = handle.state::<crate::App>();
            let (store, categories, live) =
                (app.store.clone(), app.categories.clone(), app.live.clone());
            for _ in 0..5 {
                let at = Instant::now();
                let read = crate::downloads::read_snapshot(&store, &categories, &live);
                eprintln!(
                    "zajil-measure: list of {} read in {:.1} ms",
                    read.map(|s| s.downloads.len()).unwrap_or(0),
                    at.elapsed().as_secs_f64() * 1000.0
                );
            }
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
        let progress = crate::windows::progress_label(999_999);
        for _ in 0..ROUNDS {
            for (label, size) in [
                (crate::windows::LINK, crate::windows::LINK_SIZE),
                (crate::windows::INFO, crate::windows::INFO_SIZE),
                (progress.as_str(), crate::windows::PROGRESS_SIZE),
            ] {
                if label == crate::windows::INFO {
                    use tauri::Manager;
                    handle
                        .state::<crate::windows::Shared>()
                        .lock()
                        .unwrap()
                        .set_info(
                            label,
                            crate::windows::InfoStart {
                                // `ZAJIL_MEASURE_URL`: a server that
                                // answers slowly shows what the window does
                                // meanwhile.
                                url: std::env::var("ZAJIL_MEASURE_URL")
                                    .unwrap_or_else(|_| "http://127.0.0.1:9/measure.bin".into()),
                                offer: None,
                            },
                        );
                }
                if let Err(err) = crate::windows::open(&handle, label, "Zajil", size) {
                    eprintln!("zajil-measure: {label} did not open: {err}");
                }
                // In front of every app, as a window the user opened is:
                // macOS keeps a hidden page asleep while its window is
                // covered.
                if let Some(window) = crate::windows::find(&handle, label) {
                    let _ = window.set_always_on_top(true);
                }
                tokio::time::sleep(Duration::from_millis(1500)).await;
                if let Some(window) = crate::windows::find(&handle, label) {
                    let _ = window.close();
                }
                tokio::time::sleep(Duration::from_millis(1500)).await;
            }
        }
        eprintln!("zajil-measure: done");
    });
}
