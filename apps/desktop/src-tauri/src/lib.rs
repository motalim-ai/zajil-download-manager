//! Zajil's desktop interface (M4, ADR-0003, ADR-0008): a Tauri 2 window
//! over the engine. Like the command line, it holds no download logic
//! (ARCHITECTURE §1): it runs the queues with `zajil_core::queue::Runner`,
//! sends the user's commands to the queues file, and shows what happens.
//!
//! The page asks for a [`view::Snapshot`] when told that something changed
//! (event `downloads-changed`, at most four times a second).

#![forbid(unsafe_code)]

mod live;
mod tray;
#[cfg(feature = "ui-test")]
mod ui_test;
pub mod view;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager, State};
use zajil_core::category::{CategoriesFile, CategoryStore};
use zajil_core::queue::{
    DEFAULT_QUEUE, QueueEvent, QueueStore, QueuesFile, Runner, Status, StoreError,
};
use zajil_core::settings::SettingsStore;
use zajil_core::state::DownloadState;
use zajil_core::{Engine, Options, Stop};
use zajil_platform::native_host::{Browser, NativeHostRegistrar, SystemRegistrar};
use zajil_platform::{AppPaths, SystemPaths};

use crate::live::{Live, NOTIFY_EVERY, Notify};
use crate::view::{
    AddRequest, CategoryView, DetailsView, ImportView, ProbeView, RefreshProblem, SettingsChange,
    SettingsView, Snapshot,
};

/// How long the program waits for the downloads to record their progress
/// when it is closed.
const STOP_WAIT: Duration = Duration::from_secs(5);

/// Everything the commands share.
pub struct App {
    engine: Engine,
    store: QueueStore,
    categories: CategoryStore,
    settings: SettingsStore,
    /// The system's downloads folder, used when the settings name none.
    system_download_dir: PathBuf,
    /// Connections for the next download (the settings, M4).
    connections: Arc<AtomicUsize>,
    /// Clipboard monitoring is on (the settings, M5).
    clipboard: Arc<AtomicBool>,
    /// The data folder (`--data-dir` or the system's).
    data_dir: PathBuf,
    /// The browser's cookies and page of each download the extension sent,
    /// by item: in memory only (owner's decision on #7).
    from_browser: zajil_core::queue::FromBrowserById,
    /// How the last browser registration went (at the start, or the
    /// settings button), for the settings page.
    integration: Arc<std::sync::Mutex<Option<view::IntegrationView>>>,
    live: Arc<Live>,
    stop: Stop,
    /// Set once the queues have stopped (their downloads recorded their
    /// progress), so the program may exit.
    stopped: tokio::sync::watch::Sender<bool>,
    /// The first exit request is being handled.
    exiting: AtomicBool,
    /// The page has asked for its first snapshot.
    page_ready: AtomicBool,
    /// The add dialog's probe in progress, given up when the dialog closes
    /// (review of #48).
    probing: std::sync::Mutex<Option<Stop>>,
    /// The downloads whose link is being refreshed: not resumed meanwhile.
    refreshing: Refreshing,
}

/// Where the queues and categories live: `--data-dir <folder>` (as for
/// `zajil`), or the system data folder.
fn data_dir(args: &[String]) -> Result<PathBuf, String> {
    if let Some(i) = args.iter().position(|a| a == "--data-dir")
        && let Some(dir) = args.get(i + 1)
    {
        return Ok(PathBuf::from(dir));
    }
    Ok(SystemPaths::new()
        .map_err(|e| e.to_string())?
        .data_dir()
        .to_path_buf())
}

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let data = data_dir(&args).unwrap_or_else(|e| panic!("no data folder: {e}"));
    let system_download_dir = SystemPaths::new()
        .map(|p| p.download_dir().to_path_buf())
        .unwrap_or_else(|_| data.join("Downloads"));
    let settings = SettingsStore::new(&data);
    // A damaged settings file is shown when the page asks for it; the
    // downloads run with the defaults meanwhile.
    let loaded = settings.load();
    let connections = loaded
        .as_ref()
        .map(|s| s.connections as usize)
        .unwrap_or(zajil_core::DEFAULT_CONNECTIONS);
    // Off with a damaged settings file, as by default.
    let clipboard = loaded.as_ref().is_ok_and(|s| s.clipboard);
    let app = App {
        engine: Engine::new(Options::default()).expect("the engine starts"),
        store: QueueStore::new(&data),
        categories: CategoryStore::new(&data),
        settings,
        system_download_dir,
        connections: Arc::new(AtomicUsize::new(connections)),
        clipboard: Arc::new(AtomicBool::new(clipboard)),
        data_dir: data.clone(),
        from_browser: Arc::default(),
        integration: Arc::default(),
        live: Arc::new(Live::default()),
        stop: Stop::new(),
        stopped: tokio::sync::watch::Sender::new(false),
        exiting: AtomicBool::new(false),
        page_ready: AtomicBool::new(false),
        probing: std::sync::Mutex::new(None),
        refreshing: Refreshing::default(),
    };
    #[cfg(unix)]
    let signals = stop_signals();
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(app);
    #[cfg(feature = "ui-test")]
    let builder = builder.manage(ui_test::UiTest::from_args(&args));
    builder
        .invoke_handler(handlers())
        .setup(move |tauri_app| {
            let handle = tauri_app.handle().clone();
            #[cfg(unix)]
            if let Some(signals) = signals {
                tauri::async_runtime::spawn(exit_on_signal(handle.clone(), signals));
            }
            // Not fatal: without it, a Windows shutdown still reaches the
            // stop path through tao's own handling (`RunEvent::Exit`).
            #[cfg(windows)]
            if let Err(err) = on_windows_session_end(tauri_app) {
                eprintln!("zajil: cannot watch for the end of the Windows session: {err}");
            }
            tray::build(tauri_app)?;
            serve_extension(tauri_app.handle().clone(), data.clone());
            watch_clipboard(tauri_app.handle().clone());
            // The browser integration, at each start: idempotent, and a
            // browser installed since then finds Zajil. Not in a ui-test
            // scene, whose files are demo ones.
            if !in_scene(tauri_app.handle()) {
                let data = data.clone();
                let kept = tauri_app.state::<App>().integration.clone();
                std::thread::spawn(move || {
                    let integration = integrate_browsers(&data);
                    for problem in integration_problems(&integration) {
                        eprintln!("zajil: the browser extension: {problem}");
                    }
                    *kept.lock().unwrap_or_else(|e| e.into_inner()) = Some(integration);
                });
            }
            #[cfg(feature = "ui-test")]
            if tauri_app.state::<ui_test::UiTest>().scenario.is_some() {
                // Fixed demo data: nothing runs, so nothing to wait for.
                tauri_app.state::<App>().stopped.send_replace(true);
                if let Some(window) = tauri_app.get_webview_window("main") {
                    ui_test::fit_to_screen(&window)?;
                }
                return Ok(());
            }
            tauri::async_runtime::spawn(async move {
                run_queues(handle.clone()).await;
                handle.state::<App>().stopped.send_replace(true);
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("the window opens")
        .run(|handle, event| match event {
            tauri::RunEvent::ExitRequested { api, .. } => {
                let app = handle.state::<App>();
                // The second request is ours, below: let it exit.
                if app.exiting.swap(true, Ordering::SeqCst) {
                    return;
                }
                // The downloads stop after recording their progress: the
                // program waits for the queues, then exits.
                api.prevent_exit();
                app.stop.request();
                let stopped = app.stopped.subscribe();
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    wait_for_queues(stopped).await;
                    handle.exit(0);
                });
            }
            // Quitting that bypasses the request above: on macOS, Cmd+Q, the
            // Dock's Quit, a logout and a shutdown end the program at once
            // (review of #45). The downloads are stopped here, the same way,
            // holding the exit until they have recorded their progress.
            tauri::RunEvent::Exit => {
                let app = handle.state::<App>();
                // Already stopped, or already waited for by the request
                // above: 5 s at most, never twice (review of #45).
                if *app.stopped.borrow() || app.exiting.load(Ordering::SeqCst) {
                    return;
                }
                app.exiting.store(true, Ordering::SeqCst);
                app.stop.request();
                tauri::async_runtime::block_on(wait_for_queues(app.stopped.subscribe()));
            }
            _ => {}
        });
}

// Generic over the runtime, so a test can run them on Tauri's mock runtime.
#[cfg(not(feature = "ui-test"))]
fn handlers<R: tauri::Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        snapshot,
        probe,
        cancel_probe,
        add,
        pause,
        resume,
        remove,
        details,
        set_tray_texts,
        settings,
        save_settings,
        categories,
        save_categories,
        try_category,
        export_queues,
        import_queues,
        show_window,
        fix_browser_integration,
        browser_integration,
        refresh_link
    ]
}

#[cfg(feature = "ui-test")]
fn handlers<R: tauri::Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        snapshot,
        probe,
        cancel_probe,
        add,
        pause,
        resume,
        remove,
        details,
        set_tray_texts,
        settings,
        save_settings,
        categories,
        save_categories,
        try_category,
        export_queues,
        import_queues,
        show_window,
        fix_browser_integration,
        browser_integration,
        refresh_link,
        ui_test::ui_test_scenario,
        ui_test::ui_test_report
    ]
}

/// Waits for the queues to stop, [`STOP_WAIT`] at most, and says how it
/// ended. After that it gives up: every download's progress is in its state
/// file, at most a second old (ADR-0004), and the next start resumes there.
async fn wait_for_queues(mut stopped: tokio::sync::watch::Receiver<bool>) {
    let wait = stopped.wait_for(|stopped| *stopped);
    match tokio::time::timeout(STOP_WAIT, wait).await {
        Ok(_) => eprintln!("zajil: the downloads stopped; exiting"),
        Err(_) => eprintln!(
            "zajil: the downloads did not stop within 5 s; exiting, to resume from their state files"
        ),
    }
}

/// The signals that ask a program to stop (SIGTERM: a shutdown on Linux,
/// `kill`; SIGINT: Ctrl+C in a terminal; SIGHUP: its terminal closed). On
/// Windows the system closes the window, or ends the session
/// (`on_windows_session_end`, #49).
#[cfg(unix)]
struct StopSignals {
    terminate: tokio::signal::unix::Signal,
    interrupt: tokio::signal::unix::Signal,
    hangup: tokio::signal::unix::Signal,
}

/// Registered before the window opens: a SIGTERM that comes early still
/// closes the program cleanly (one came before the handler was ready in
/// CI, on #46, and killed it). `None`, said on stderr, if the system
/// refuses.
#[cfg(unix)]
fn stop_signals() -> Option<StopSignals> {
    use tokio::signal::unix::{SignalKind, signal};
    tauri::async_runtime::block_on(async {
        Ok::<_, std::io::Error>(StopSignals {
            terminate: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
            hangup: signal(SignalKind::hangup())?,
        })
    })
    .inspect_err(|err| eprintln!("zajil: signals will not close the program cleanly: {err}"))
    .ok()
}

/// Exits as when the window is closed, so the downloads record their
/// progress, and with code 0. A second signal while it waits exits at once
/// (review of #45: it was ignored), with code 1: the progress of the last
/// second may be lost, never a file.
#[cfg(unix)]
async fn exit_on_signal(handle: tauri::AppHandle, mut signals: StopSignals) {
    let mut asked = false;
    loop {
        let name = tokio::select! {
            _ = signals.terminate.recv() => "SIGTERM",
            _ = signals.interrupt.recv() => "SIGINT",
            _ = signals.hangup.recv() => "SIGHUP",
        };
        if asked {
            eprintln!("zajil: asked again ({name}); exiting now");
            std::process::exit(1);
        }
        asked = true;
        eprintln!("zajil: asked to stop ({name})");
        handle.exit(0);
    }
}

/// Windows has no SIGTERM: at a shutdown, restart or logoff it sends the
/// window `WM_ENDSESSION`, and may end the process once that is answered
/// (#49). The downloads stop there, after recording their progress, as
/// when the window is closed; then the program exits.
#[cfg(windows)]
fn on_windows_session_end(tauri_app: &tauri::App) -> tauri::Result<()> {
    let Some(window) = tauri_app.get_webview_window("main") else {
        return Ok(());
    };
    let handle = tauri_app.handle().clone();
    let hwnd = window.hwnd()?.0 as isize;
    zajil_platform::session::on_session_end(
        hwnd,
        Box::new(move || {
            eprintln!("zajil: asked to stop (the Windows session is ending)");
            let app = handle.state::<App>();
            if !app.exiting.swap(true, Ordering::SeqCst) {
                app.stop.request();
            }
            tauri::async_runtime::block_on(wait_for_queues(app.stopped.subscribe()));
            std::process::exit(0);
        }),
    )
    .map_err(tauri::Error::Io)
}

/// The tray menu's texts in the interface's language.
#[tauri::command]
fn set_tray_texts<R: tauri::Runtime>(
    handle: tauri::AppHandle<R>,
    show: String,
    quit: String,
) -> Result<(), String> {
    tray::set_texts(&handle, &show, &quit).map_err(|e| e.to_string())
}

/// Tells the page that something changed: at once, or once the throttle
/// allows, so the last change of a burst is never lost.
fn notify(handle: &tauri::AppHandle, live: &Arc<Live>) {
    match live.notify_at(Instant::now()) {
        Notify::Now => {
            let _ = handle.emit("downloads-changed", ());
        }
        Notify::Later(wait) => {
            let (handle, live) = (handle.clone(), live.clone());
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(wait.min(NOTIFY_EVERY)).await;
                live.notified_later(Instant::now());
                let _ = handle.emit("downloads-changed", ());
            });
        }
        Notify::Scheduled => {}
    }
}

/// Runs the queues for as long as the window is open. The runner ends when
/// nothing is left to download; it starts again as soon as something is.
async fn run_queues(handle: tauri::AppHandle) {
    let (engine, store, categories, connections, live, stop, from_browser) = {
        let app = handle.state::<App>();
        (
            app.engine.clone(),
            app.store.clone(),
            app.categories.clone(),
            app.connections.clone(),
            app.live.clone(),
            app.stop.clone(),
            app.from_browser.clone(),
        )
    };
    loop {
        if stop.is_requested() {
            return;
        }
        let runner = queue_runner(&engine, &store, &categories, &connections, &from_browser);
        let result = runner
            .run(&stop, &mut |event: QueueEvent| {
                live.record(&event);
                notify(&handle, &live);
            })
            .await;
        if let Err(err) = result {
            // The queues file is damaged or unreadable: say it, and look
            // again later (the user may move the file away). Nothing runs.
            live.clear();
            live.set_problem(Some(err.to_string()));
            let _ = handle.emit("downloads-changed", ());
        }
        // Wait until something is waiting again, or the window closes.
        loop {
            tokio::select! {
                () = stop.requested() => return,
                () = tokio::time::sleep(Duration::from_millis(500)) => {}
            }
            let store = store.clone();
            let waiting = tokio::task::spawn_blocking(move || {
                store.load().map_err(|e| e.to_string()).map(|q| {
                    q.queues
                        .iter()
                        .flat_map(|q| &q.items)
                        .any(|i| matches!(i.status, Status::Waiting | Status::Active))
                })
            })
            .await
            .unwrap_or_else(|e| Err(format!("reading the queues file failed: {e}")));
            match waiting {
                Ok(waiting) => {
                    // A file the user fixed: its problem goes (review of #45).
                    if live.problem().is_some() {
                        live.set_problem(None);
                        let _ = handle.emit("downloads-changed", ());
                    }
                    if waiting {
                        break;
                    }
                }
                Err(err) => {
                    live.set_problem(Some(err));
                    let _ = handle.emit("downloads-changed", ());
                }
            }
        }
    }
}

fn blocking_err(err: impl std::fmt::Display) -> String {
    err.to_string()
}

#[tauri::command]
async fn snapshot<R: tauri::Runtime>(
    app: State<'_, App>,
    handle: tauri::AppHandle<R>,
) -> Result<Snapshot, String> {
    if !app.page_ready.swap(true, Ordering::SeqCst) {
        // The page is up and talking to the program (the open-and-close
        // check of CI waits for this line).
        eprintln!("zajil: the window is ready");
    }
    #[cfg(feature = "ui-test")]
    if let Some(snapshot) = handle.state::<ui_test::UiTest>().demo() {
        return Ok(snapshot);
    }
    #[cfg(not(feature = "ui-test"))]
    let _ = handle;
    let (store, categories, live) = (app.store.clone(), app.categories.clone(), app.live.clone());
    tokio::task::spawn_blocking(move || read_snapshot(&store, &categories, &live))
        .await
        .map_err(blocking_err)?
}

#[tauri::command]
async fn probe(app: State<'_, App>, url: String) -> Result<ProbeView, String> {
    // What adding would refuse is refused before the server is asked
    // (review of #48: a URL with a password passed, then `add` refused it).
    zajil_core::queue::check_url(&url).map_err(|e| e.to_string())?;
    // A new probe takes the place of one still running.
    let stop = Stop::new();
    if let Some(old) = app.probing.lock().unwrap().replace(stop.clone()) {
        old.request();
    }
    let found = app
        .engine
        .probe_until(&url, &stop)
        .await
        .map_err(|e| e.to_string())?;
    let (categories, settings, system) = (
        app.categories.clone(),
        app.settings.clone(),
        app.system_download_dir.clone(),
    );
    tokio::task::spawn_blocking(move || {
        // A damaged categories file: none, as in the list, so a download
        // can still be added (review of #48).
        let categories = categories.load().unwrap_or_else(|_| no_categories());
        let given = download_dir(&settings, &system);
        view::probe(&found, &categories, &given)
    })
    .await
    .map_err(blocking_err)
}

/// Gives up the add dialog's probe (the dialog was closed).
#[tauri::command]
fn cancel_probe(app: State<'_, App>) {
    if let Some(stop) = app.probing.lock().unwrap().take() {
        stop.request();
    }
}

/// What a damaged categories file is read as: no categories at all (the
/// list says the problem itself).
fn no_categories() -> CategoriesFile {
    let mut none = CategoriesFile::built_in();
    none.categories.clear();
    none
}

#[tauri::command]
async fn add<R: tauri::Runtime>(
    app: State<'_, App>,
    request: AddRequest,
    handle: tauri::AppHandle<R>,
) -> Result<u64, String> {
    // A `ui-test` scene shows demo data and never touches the real queues
    // (as `change`, review of #47).
    #[cfg(feature = "ui-test")]
    if handle.state::<ui_test::UiTest>().scenario.is_some() {
        return Err("a ui-test scene adds nothing".into());
    }
    let (store, settings, system) = (
        app.store.clone(),
        app.settings.clone(),
        app.system_download_dir.clone(),
    );
    let id = tokio::task::spawn_blocking(move || {
        let given = chosen_folder(request.folder.as_deref(), &download_dir(&settings, &system))
            .map_err(StoreError::Refused)?;
        add_blocking(&store, &request, &given)
    })
    .await
    .map_err(blocking_err)?
    .map_err(|e| e.to_string())?;
    let _ = handle.emit("downloads-changed", ());
    Ok(id)
}

/// The folder a new download goes to: its own, checked as a category's
/// folder is (absolute, no `..`, valid text without hidden characters),
/// since it comes from the page; or the given download folder.
fn chosen_folder(folder: Option<&str>, download_dir: &Path) -> Result<PathBuf, String> {
    match folder {
        Some(folder) => {
            let folder = PathBuf::from(folder);
            zajil_core::category::check_folder(&folder).map_err(|e| e.to_string())?;
            Ok(folder)
        }
        None => Ok(download_dir.to_path_buf()),
    }
}

/// The browser extension's requests, handed over by `zajil-host` on the
/// user's private channel (ADR-0005, M5), served by `zajil_host::serve`.
/// A second app on the same data folder cannot open the channel: it says
/// so and goes on without it.
fn serve_extension<R: tauri::Runtime>(handle: tauri::AppHandle<R>, data: PathBuf) {
    let listener = match zajil_platform::ipc::Listener::bind(&data) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("zajil: the browser extension cannot reach this window: {err}");
            return;
        }
    };
    std::thread::spawn(move || {
        zajil_host::serve(listener, move |request| {
            // In a ui-test scene, nothing reaches the queues.
            if in_scene(&handle) {
                return zajil_host::Response::refused("a ui-test scene adds nothing");
            }
            let app = handle.state::<App>();
            let given = download_dir(&app.settings, &app.system_download_dir);
            let reach = extension_reach(&app.engine);
            let answer = from_extension(&app.store, &request, &given, &app.from_browser, reach);
            if answer.ok {
                let _ = handle.emit("downloads-changed", ());
                tray::show_window(&handle);
            }
            answer
        });
    });
}

/// How often the clipboard is looked at while monitoring is on.
const CLIPBOARD_EVERY: std::time::Duration = std::time::Duration::from_millis(500);

/// The least time between two links offered: a program changing the
/// clipboard all the time cannot raise the window twice a second.
const CLIPBOARD_OFFER_GAP: std::time::Duration = std::time::Duration::from_secs(2);

/// Clipboard monitoring (M5, off unless the user turns it on): while the
/// setting is on, the clipboard is looked at twice a second (and read only
/// after a change where the system says so), and a download link the user
/// copies goes to the page (`clipboard-link`). The page puts it in the
/// paste bar and shows the window, unless the add dialog is open: **no
/// request reaches the link before the user presses "Download"**, and
/// nothing is added before the user agrees. While it is off the clipboard
/// is not opened; in a ui-test scene it stays off.
fn watch_clipboard<R: tauri::Runtime>(handle: tauri::AppHandle<R>) {
    std::thread::spawn(move || {
        let mut watcher = ClipboardWatcher::default();
        loop {
            std::thread::sleep(CLIPBOARD_EVERY);
            let app = handle.state::<App>();
            let on = app.clipboard.load(Ordering::Relaxed) && !in_scene(&handle);
            let link = watcher.tick(
                on,
                zajil_platform::clipboard::SystemClipboard::new,
                &app.categories,
                std::time::Instant::now(),
            );
            if let Some(link) = link {
                let _ = handle.emit("clipboard-link", link);
            }
        }
    });
}

/// `zajil-host` beside the program `exe` (the installers put it there; in a
/// development build, `cargo build -p zajil-host` run in
/// `apps/desktop/src-tauri` does), if a browser may safely be told to start
/// it: `exe`'s real path (links resolved) is not in a place that goes away
/// when the program closes, and on macOS and Linux the host is a program
/// (an execute bit) that only its owner, the user or root, can change, in a
/// folder only they can change.
fn host_beside(exe: &Path) -> Result<PathBuf, view::HostProblemView> {
    use view::HostProblemView as Problem;
    // Links resolved on macOS and Linux (a CLI or Homebrew link). Not on
    // Windows: there the real path takes a `\\?\` form browsers may not
    // start, and the installers make no links.
    #[cfg(unix)]
    let exe = std::fs::canonicalize(exe).map_err(|_| Problem::Missing)?;
    #[cfg(windows)]
    let exe = exe.to_path_buf();
    let temporary = exe.components().any(|part| {
        let part = part.as_os_str().to_string_lossy();
        part.starts_with(".mount_") || part == "AppTranslocation"
    });
    if temporary || std::env::var_os("APPIMAGE").is_some() {
        return Err(Problem::Temporary);
    }
    let dir = exe.parent().ok_or(Problem::Missing)?;
    let host = dir.join(if cfg!(windows) {
        "zajil-host.exe"
    } else {
        "zajil-host"
    });
    let meta = host.symlink_metadata().map_err(|_| Problem::Missing)?;
    if !meta.is_file() {
        return Err(Problem::Missing);
    }
    if meta.len() == 0 {
        return Err(Problem::Missing);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err(Problem::Missing);
        }
    }
    let safe =
        |path: &Path| zajil_platform::fs::only_user_or_root_can_change(path).unwrap_or(false);
    if !safe(&host) || !safe(dir) {
        return Err(Problem::Unsafe);
    }
    Ok(host)
}

/// Registers `zajil-host` with every supported browser for the current user
/// (ADR-0005), each allowing only Zajil's extension: its fixed id in
/// Chrome, Chromium and Edge, its Firefox id in Firefox. One registration
/// at a time in this program (the start and the settings button).
fn integrate_browsers(data: &Path) -> view::IntegrationView {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let host = std::env::current_exe()
        .map_err(|_| view::HostProblemView::Missing)
        .and_then(|exe| host_beside(&exe));
    integrate_with(host, || SystemRegistrar::for_current_user(data))
}

/// [`integrate_browsers`] with the host found (or why not) and a way to
/// make the registrar, asked for only when there is a host.
fn integrate_with<G: NativeHostRegistrar>(
    host: Result<PathBuf, view::HostProblemView>,
    registrar: impl FnOnce() -> std::io::Result<G>,
) -> view::IntegrationView {
    let host = match host {
        Ok(host) => host,
        Err(problem) => {
            return view::IntegrationView {
                host_problem: Some(problem),
                browsers: Vec::new(),
            };
        }
    };
    match registrar() {
        Ok(registrar) => register_all(&registrar, &host),
        Err(err) => view::IntegrationView {
            host_problem: None,
            browsers: Browser::ALL
                .iter()
                .map(|b| view::BrowserIntegrationView {
                    browser: browser_name(*b).to_owned(),
                    error: Some(err.to_string()),
                })
                .collect(),
        },
    }
}

/// [`integrate_browsers`] with a given registrar and host.
fn register_all(registrar: &impl NativeHostRegistrar, host: &Path) -> view::IntegrationView {
    use zajil_platform::ids::{EXTENSION_ID_CHROMIUM, EXTENSION_ID_FIREFOX};
    let browsers = Browser::ALL
        .iter()
        .map(|&browser| {
            let allowed = if browser == Browser::Firefox {
                EXTENSION_ID_FIREFOX
            } else {
                EXTENSION_ID_CHROMIUM
            };
            view::BrowserIntegrationView {
                browser: browser_name(browser).to_owned(),
                error: registrar
                    .register(browser, host, &[allowed])
                    .err()
                    .map(|e| e.to_string()),
            }
        })
        .collect();
    view::IntegrationView {
        host_problem: None,
        browsers,
    }
}

fn browser_name(browser: Browser) -> &'static str {
    match browser {
        Browser::Chrome => "chrome",
        Browser::Chromium => "chromium",
        Browser::Edge => "edge",
        Browser::Firefox => "firefox",
    }
}

/// What went wrong, in words for the log.
fn integration_problems(integration: &view::IntegrationView) -> Vec<String> {
    use view::HostProblemView as Problem;
    match integration.host_problem {
        Some(Problem::Missing) => {
            return vec![
                "zajil-host is not next to the program, so no browser can reach Zajil".into(),
            ];
        }
        Some(Problem::Temporary) => {
            return vec![
                "the program runs from a place that goes away when it closes, so it registers nothing"
                    .into(),
            ];
        }
        Some(Problem::Unsafe) => {
            return vec![
                "zajil-host or its folder can be changed by another user, so it is not registered"
                    .into(),
            ];
        }
        None => {}
    }
    integration
        .browsers
        .iter()
        .filter_map(|b| {
            Some(format!(
                "cannot register with {}: {}",
                b.browser,
                b.error.as_ref()?
            ))
        })
        .collect()
}

/// "Fix the browser integration" (settings): registers again, and says
/// how it went for each browser.
#[tauri::command]
async fn fix_browser_integration<R: tauri::Runtime>(
    app: State<'_, App>,
    handle: tauri::AppHandle<R>,
) -> Result<view::IntegrationView, String> {
    if in_scene(&handle) {
        return Err("a ui-test scene registers nothing".into());
    }
    let (data, kept) = (app.data_dir.clone(), app.integration.clone());
    let integration = tokio::task::spawn_blocking(move || integrate_browsers(&data))
        .await
        .map_err(blocking_err)?;
    *kept.lock().unwrap_or_else(|e| e.into_inner()) = Some(integration.clone());
    Ok(integration)
}

/// How the last browser registration went (`null` while the one at the
/// start is still running, or in a ui-test scene).
#[tauri::command]
fn browser_integration(app: State<'_, App>) -> Option<view::IntegrationView> {
    app.integration
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Shows the window and brings it to the front: the page asks when it has
/// taken a link from the clipboard.
#[tauri::command]
fn show_window<R: tauri::Runtime>(handle: tauri::AppHandle<R>) {
    tray::show_window(&handle);
}

/// Clipboard monitoring between two reads.
struct ClipboardWatcher<C> {
    /// Open while monitoring is on.
    clipboard: Option<C>,
    watch: zajil_core::clipboard::Watch,
    /// When the last link was offered.
    offered: Option<std::time::Instant>,
    /// The last problem said, so a clipboard that cannot be read is not
    /// reported twice a second.
    said: Option<String>,
}

impl<C> Default for ClipboardWatcher<C> {
    fn default() -> Self {
        ClipboardWatcher {
            clipboard: None,
            watch: Default::default(),
            offered: None,
            said: None,
        }
    }
}

impl<C: zajil_platform::clipboard::Clipboard> ClipboardWatcher<C> {
    /// One look at `now`: the link to offer, if the user copied one since
    /// the last. An unchanged clipboard is not read; text longer than
    /// `MAX_TEXT` is not kept; a link copied within
    /// [`CLIPBOARD_OFFER_GAP`] of the last one offered is not offered.
    /// `on` false closes the clipboard and forgets what was seen, so what
    /// is on the clipboard when monitoring is turned on again is not
    /// offered. A clipboard that cannot be opened or read (another program
    /// holding it, no display) is tried again at the next look.
    fn tick(
        &mut self,
        on: bool,
        open: impl FnOnce() -> std::io::Result<C>,
        categories: &CategoryStore,
        now: std::time::Instant,
    ) -> Option<String> {
        if !on {
            *self = ClipboardWatcher::default();
            return None;
        }
        if self.clipboard.is_none() {
            match open() {
                Ok(clipboard) => self.clipboard = Some(clipboard),
                Err(err) => return self.say(err),
            }
        }
        let clipboard = self.clipboard.as_mut()?;
        if !clipboard.changed() {
            return None;
        }
        let content = match clipboard.text(zajil_core::clipboard::MAX_TEXT) {
            Ok(content) => content,
            Err(err) => {
                self.clipboard = None;
                return self.say(err);
            }
        };
        self.said = None;
        let link = self.watch.seen(&content, || categories.load().ok())?;
        if self
            .offered
            .is_some_and(|last| now.duration_since(last) < CLIPBOARD_OFFER_GAP)
        {
            return None;
        }
        self.offered = Some(now);
        Some(link)
    }

    fn say(&mut self, err: std::io::Error) -> Option<String> {
        let said = err.to_string();
        if self.said.as_ref() != Some(&said) {
            eprintln!("zajil: clipboard monitoring: {said}");
            self.said = Some(said);
        }
        None
    }
}

/// How long the app may take to reach the file before it answers the
/// extension: within `zajil_host::CHANNEL_TIMEOUT` (10 s), so the host
/// still has the answer.
const EXTENSION_REACH: Duration = Duration::from_secs(7);

/// The check of a download from the extension: the engine, with the
/// browser's cookies and page for this download.
fn extension_reach(
    engine: &Engine,
) -> impl Fn(&str, &zajil_core::FromBrowser) -> Result<zajil_core::Probe, String> + '_ {
    move |url, from| reach_file(&engine.with_browser(from.clone()), url)
}

/// The window's queue runner: categories, the settings' connections, and
/// the browser's headers of each download the extension sent.
fn queue_runner(
    engine: &Engine,
    store: &QueueStore,
    categories: &CategoryStore,
    connections: &Arc<AtomicUsize>,
    from_browser: &zajil_core::queue::FromBrowserById,
) -> Runner {
    Runner::new(engine.clone(), store.clone())
        .categories(categories.clone())
        .connections(connections.clone())
        .from_browser(from_browser.clone())
}

/// What the engine reaches at `url` now, as the add dialog does (the first
/// response's headers only), within [`EXTENSION_REACH`].
fn reach_file(engine: &Engine, url: &str) -> Result<zajil_core::Probe, String> {
    let stop = Stop::new();
    tauri::async_runtime::block_on(async {
        match tokio::time::timeout(EXTENSION_REACH, engine.probe_until(url, &stop)).await {
            Ok(Ok(probe)) => Ok(probe),
            Ok(Err(err)) => Err(err.to_string()),
            Err(_) => {
                stop.request();
                Err("the server did not answer in time".to_owned())
            }
        }
    })
}

/// Why what Zajil reached is not the file the browser was downloading: a
/// web page (a sign-in page a link redirected to, review of #70), or a file
/// of another type than the name the browser had.
fn not_the_file(probe: &zajil_core::Probe, browser_name: Option<&str>) -> Option<String> {
    let page = matches!(
        probe.content_type.as_deref(),
        Some("text/html" | "application/xhtml+xml")
    );
    if page {
        return Some("the server sent a web page (a sign-in page?), not the file".into());
    }
    let extension = |name: &str| {
        name.rsplit_once('.')
            .map(|(_, ext)| ext.to_lowercase())
            .filter(|ext| !ext.is_empty())
    };
    let wanted = browser_name.and_then(extension)?;
    (extension(&probe.name).as_deref() != Some(wanted.as_str())).then(|| {
        format!(
            "the server sent another file ({}), not a .{wanted}",
            probe.name
        )
    })
}

/// What a request from the extension does: a download goes into the
/// default queue, by category, as the add dialog adds one, **once Zajil
/// has reached the file itself** (`reach`), and it is that file (not a web
/// page, the same extension as the browser's name): otherwise it is
/// refused, and the extension leaves it to the browser (review of #70: a
/// file behind a login, a referrer check or a one-time link would vanish
/// from the browser and then fail in Zajil). The cookies (sent only when
/// the user allowed them in the extension) and the page go with the check
/// and the download, from memory (`App::from_browser`).
fn from_extension(
    store: &QueueStore,
    request: &zajil_host::Request,
    given: &Path,
    headers: &zajil_core::queue::FromBrowserById,
    reach: impl FnOnce(&str, &zajil_core::FromBrowser) -> Result<zajil_core::Probe, String>,
) -> zajil_host::Response {
    match request {
        zajil_host::Request::Ping { .. } => zajil_host::Response::ok(),
        zajil_host::Request::Download {
            url,
            filename,
            referrer,
            cookies,
            ..
        } => {
            if let Err(err) = zajil_core::queue::check_url(url) {
                return zajil_host::Response::refused(err.to_string());
            }
            // The browser's cookies (when the user allowed them) and page,
            // for the check and then the download: in memory only.
            let from = zajil_core::FromBrowser::new(url, cookies.as_deref(), referrer.as_deref());
            let why = match reach(url, &from) {
                Ok(probe) => not_the_file(&probe, filename.as_deref()),
                Err(why) => Some(why),
            };
            if let Some(why) = why {
                return zajil_host::Response::refused(format!("Zajil cannot download it: {why}"));
            }
            let add = AddRequest {
                url: url.clone(),
                queue: None,
                folder: None,
                start: true,
                limit: None,
            };
            match add_blocking(store, &add, given) {
                Ok(id) => {
                    headers
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(id, from);
                    zajil_host::Response::ok()
                }
                Err(err) => zajil_host::Response::refused(err.to_string()),
            }
        }
    }
}

/// Nothing is created here: the download makes its folder when it starts,
/// so a refused add leaves nothing behind (review of #48).
fn add_blocking(store: &QueueStore, request: &AddRequest, given: &Path) -> Result<u64, StoreError> {
    store.update(|queues| {
        let queue = request.queue.as_deref().unwrap_or(DEFAULT_QUEUE);
        let id = queues.add(queue, &request.url, given)?;
        let item = queues.item_mut(id).expect("just added");
        // Without a folder of its own, a download goes by its category.
        item.categorize = request.folder.is_none();
        item.limit = request.limit.filter(|l| *l > 0);
        if !request.start {
            item.status = Status::Paused;
        }
        Ok(id)
    })
}

/// What the page shows, read from the files. Blocking. A damaged categories
/// file is said, not fatal: the downloads still show (review of #45: the
/// whole list went blank while they ran).
fn read_snapshot(
    store: &QueueStore,
    categories: &CategoryStore,
    live: &Live,
) -> Result<Snapshot, String> {
    let queues = store.load().map_err(blocking_err)?;
    let (categories, problem) = match categories.load() {
        Ok(c) => (c, None),
        Err(err) => {
            let mut none = CategoriesFile::built_in();
            none.categories.clear();
            (none, Some(err.to_string()))
        }
    };
    let mut snapshot = view::snapshot(&queues, &categories, live);
    snapshot.problem = snapshot.problem.or(problem);
    Ok(snapshot)
}

/// Sets the status of items, as `zajil queue pause` and `resume` do: an
/// unknown id is an error and changes nothing. A running download stops
/// after recording its progress (the runner sees the change within half a
/// second).
fn set_status(queues: &mut QueuesFile, ids: &[u64], status: Status) -> Result<(), StoreError> {
    for id in ids {
        queues.item(*id).ok_or(StoreError::NoSuchItem(*id))?;
    }
    for id in ids {
        let item = queues.item_mut(*id).expect("checked above");
        let allowed = match status {
            Status::Paused => matches!(item.status, Status::Waiting | Status::Active),
            Status::Waiting => matches!(item.status, Status::Paused | Status::Failed),
            _ => false,
        };
        if allowed {
            item.status = status;
            if status == Status::Waiting {
                item.error = None;
            }
        }
    }
    Ok(())
}

/// Removes items from their queue, as `zajil queue remove` does: an unknown
/// id is an error and changes nothing. Files already downloaded stay.
fn remove_items(queues: &mut QueuesFile, ids: &[u64]) -> Result<(), StoreError> {
    for id in ids {
        queues.item(*id).ok_or(StoreError::NoSuchItem(*id))?;
    }
    for id in ids {
        queues.remove(*id)?;
    }
    Ok(())
}

async fn change<R: tauri::Runtime>(
    app: &App,
    handle: &tauri::AppHandle<R>,
    change: impl FnOnce(&mut QueuesFile) -> Result<(), StoreError> + Send + 'static,
) -> Result<(), String> {
    // A `ui-test` scene shows demo data: its toolbar changes nothing, so the
    // real queues are never touched (review of #47).
    #[cfg(feature = "ui-test")]
    if handle.state::<ui_test::UiTest>().scenario.is_some() {
        return Ok(());
    }
    let store = app.store.clone();
    tokio::task::spawn_blocking(move || store.update(change))
        .await
        .map_err(blocking_err)?
        .map_err(|e| e.to_string())?;
    let _ = handle.emit("downloads-changed", ());
    Ok(())
}

#[tauri::command]
async fn pause<R: tauri::Runtime>(
    app: State<'_, App>,
    ids: Vec<u64>,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    change(&app, &handle, move |q| set_status(q, &ids, Status::Paused)).await
}

#[tauri::command]
async fn resume<R: tauri::Runtime>(
    app: State<'_, App>,
    ids: Vec<u64>,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    // A download whose link is being refreshed waits for the refresh: it
    // would otherwise start from the old link, or start over.
    let ids = app.refreshing.without(ids);
    change(&app, &handle, move |q| set_status(q, &ids, Status::Waiting)).await
}

/// "Refresh the download link" (M5): download `id` goes on from `url`.
#[tauri::command]
async fn refresh_link<R: tauri::Runtime>(
    app: State<'_, App>,
    id: u64,
    url: String,
    handle: tauri::AppHandle<R>,
) -> Result<(), RefreshProblem> {
    // A scene's demo downloads are not real (review of #47).
    #[cfg(feature = "ui-test")]
    if handle.state::<ui_test::UiTest>().scenario.is_some() {
        return Ok(());
    }
    refresh_item(
        &app.engine,
        &app.store,
        &app.from_browser,
        &app.refreshing,
        id,
        &url,
    )
    .await?;
    let _ = handle.emit("downloads-changed", ());
    Ok(())
}

/// The downloads whose link is being refreshed.
#[derive(Default)]
struct Refreshing(std::sync::Mutex<std::collections::HashSet<u64>>);

impl Refreshing {
    /// Marks `id` as being refreshed until the guard is dropped; `None`
    /// when it already is.
    fn start(&self, id: u64) -> Option<RefreshGuard<'_>> {
        let mut ids = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // Made only when marked: a guard made and dropped here would lock
        // again, and unmark the refresh under way.
        ids.insert(id).then(|| RefreshGuard { of: self, id })
    }

    /// `ids` without those being refreshed.
    fn without(&self, mut ids: Vec<u64>) -> Vec<u64> {
        let busy = self.0.lock().unwrap_or_else(|e| e.into_inner());
        ids.retain(|id| !busy.contains(id));
        ids
    }
}

struct RefreshGuard<'a> {
    of: &'a Refreshing,
    id: u64,
}

impl Drop for RefreshGuard<'_> {
    fn drop(&mut self) {
        let mut ids = self.of.0.lock().unwrap_or_else(|e| e.into_inner());
        ids.remove(&self.id);
    }
}

/// Gives download `id` a new link: a paused or failed download goes on
/// from `url` once its server shows it is the same file
/// ([`Engine::refresh_link`], with the browser's cookies and page kept
/// for it); one with nothing downloaded yet just takes the link. Then it
/// waits for its turn again.
async fn refresh_item(
    engine: &Engine,
    store: &QueueStore,
    from_browser: &zajil_core::queue::FromBrowserById,
    refreshing: &Refreshing,
    id: u64,
    url: &str,
) -> Result<(), RefreshProblem> {
    zajil_core::queue::check_url(url).map_err(|e| RefreshProblem::BadLink {
        reason: e.to_string(),
    })?;
    // As the URL parser writes it, as the engine keeps it in the state, so
    // the queue still resumes that state for the item.
    let url = zajil_core::Url::parse(url)
        .map_err(|e| RefreshProblem::BadLink {
            reason: e.to_string(),
        })?
        .to_string();
    let _guard = refreshing.start(id).ok_or(RefreshProblem::NotStopped)?;
    let loaded = store.clone();
    let queues = tokio::task::spawn_blocking(move || loaded.load())
        .await
        .map_err(|e| RefreshProblem::Failed {
            error: e.to_string(),
        })?
        .map_err(|e| RefreshProblem::Failed {
            error: e.to_string(),
        })?;
    let item = queues
        .item(id)
        .ok_or_else(|| RefreshProblem::Failed {
            error: format!("no download {id}"),
        })?
        .clone();
    match item.status {
        Status::Paused | Status::Failed => {}
        Status::Done => return Err(RefreshProblem::Done),
        Status::Waiting | Status::Active => return Err(RefreshProblem::NotStopped),
    }
    let state_file = match item.state_file {
        Some(path) => tokio::task::spawn_blocking(move || path.exists().then_some(path))
            .await
            .unwrap_or(None),
        None => None,
    };
    if let Some(state_file) = state_file {
        let from = from_browser
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned();
        let engine = match from {
            Some(from) => engine.with_browser(from),
            None => engine.clone(),
        };
        engine.refresh_link(&state_file, &url).await?;
    }
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        store.update(|q| {
            let item = q.item_mut(id).ok_or(StoreError::NoSuchItem(id))?;
            item.url = url;
            item.error = None;
            if matches!(item.status, Status::Paused | Status::Failed) {
                item.status = Status::Waiting;
            }
            Ok(())
        })
    })
    .await
    .map_err(|e| RefreshProblem::Failed {
        error: e.to_string(),
    })?
    .map_err(|e| RefreshProblem::Failed {
        error: e.to_string(),
    })
}

#[tauri::command]
async fn remove<R: tauri::Runtime>(
    app: State<'_, App>,
    ids: Vec<u64>,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    change(&app, &handle, move |q| remove_items(q, &ids)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three() -> QueuesFile {
        let mut q = QueuesFile::empty();
        let dir = std::env::temp_dir();
        for n in 1..=3 {
            q.add("main", &format!("https://example.com/{n}.iso"), &dir)
                .unwrap();
        }
        q
    }

    #[test]
    fn pause_and_resume_change_only_what_they_may() {
        let mut q = three();
        q.item_mut(3).unwrap().status = Status::Done;
        set_status(&mut q, &[1, 3], Status::Paused).unwrap();
        assert_eq!(q.item(1).unwrap().status, Status::Paused);
        assert_eq!(q.item(3).unwrap().status, Status::Done);
        q.item_mut(2).unwrap().status = Status::Failed;
        q.item_mut(2).unwrap().error = Some("404".into());
        set_status(&mut q, &[1, 2], Status::Waiting).unwrap();
        assert_eq!(q.item(1).unwrap().status, Status::Waiting);
        assert_eq!(q.item(2).unwrap().status, Status::Waiting);
        assert!(q.item(2).unwrap().error.is_none());
    }

    /// M5: every browser is registered with `zajil-host`, each allowing
    /// only Zajil's extension (its fixed id; the Firefox id in Firefox),
    /// and nothing is left when unregistered.
    #[test]
    fn every_browser_is_registered_for_zajil_s_extension_only() {
        use zajil_platform::native_host::{Browser, NativeHostRegistrar, SystemRegistrar};
        let base = tempfile::tempdir().unwrap();
        let home = base.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        // A test registry root on Windows; only it, empty, may remain.
        let root = format!(r"Software\ZajilDesktopTest-{}", std::process::id());
        let registrar =
            SystemRegistrar::under(&home, &base.path().join("data/native-messaging"), &root);
        let host = base.path().join(if cfg!(windows) {
            "zajil-host.exe"
        } else {
            "zajil-host"
        });
        let integration = register_all(&registrar, &host);
        assert_eq!(integration.host_problem, None);
        assert_eq!(integration.browsers.len(), 4);
        for (browser, view) in Browser::ALL.iter().zip(&integration.browsers) {
            assert_eq!(view.error, None, "{browser:?}");
            assert!(registrar.is_registered(*browser).unwrap());
            let manifest = std::fs::read_to_string(registrar.manifest_path(*browser)).unwrap();
            let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
            assert_eq!(manifest["path"], host.to_str().unwrap());
            let allowed = if *browser == Browser::Firefox {
                serde_json::json!({ "allowed_extensions": [zajil_platform::ids::EXTENSION_ID_FIREFOX] })
            } else {
                serde_json::json!({ "allowed_origins": [format!("chrome-extension://{}/", zajil_platform::ids::EXTENSION_ID_CHROMIUM)] })
            };
            let (key, value) = allowed.as_object().unwrap().iter().next().unwrap();
            assert_eq!(&manifest[key], value, "{browser:?}");
        }
        assert_eq!(integration_problems(&integration), Vec::<String>::new());
        for browser in Browser::ALL {
            registrar.unregister(browser).unwrap();
        }
        let mut left = Vec::new();
        for entry in walk(base.path()) {
            if entry.is_file() {
                left.push(entry);
            }
        }
        assert_eq!(left, Vec::<PathBuf>::new());
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            }
            out.push(path);
        }
        out
    }

    /// A host that is missing, not a program, or in a temporary place is
    /// not registered: the registrar is not even made, and the log says
    /// why. A host beside the program is found through a link to it.
    #[test]
    fn a_host_that_is_missing_or_unsafe_is_not_registered() {
        use view::HostProblemView as Problem;
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("app");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("zajil-desktop");
        std::fs::write(&exe, "app").unwrap();
        let host = dir.join(if cfg!(windows) {
            "zajil-host.exe"
        } else {
            "zajil-host"
        });
        let executable = |path: &Path, yes: bool| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if yes { 0o755 } else { 0o644 };
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
            }
            let _ = (path, yes);
        };
        executable(&dir, true);
        assert_eq!(host_beside(&exe), Err(Problem::Missing), "no host");
        std::fs::create_dir(&host).unwrap();
        assert_eq!(host_beside(&exe), Err(Problem::Missing), "a folder");
        std::fs::remove_dir(&host).unwrap();
        std::fs::write(&host, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            executable(&host, false);
            assert_eq!(host_beside(&exe), Err(Problem::Missing), "not a program");
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&host, std::fs::Permissions::from_mode(0o777)).unwrap();
            assert_eq!(
                host_beside(&exe),
                Err(Problem::Unsafe),
                "writable by others"
            );
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
            std::fs::set_permissions(&host, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(
                host_beside(&exe),
                Err(Problem::Unsafe),
                "folder writable by others"
            );
            executable(&dir, true);
        }
        executable(&host, true);
        let found = host_beside(&exe).unwrap();
        #[cfg(unix)]
        assert_eq!(found, std::fs::canonicalize(&host).unwrap());
        #[cfg(windows)]
        assert_eq!(found, host);
        // Started through a link (a CLI or Homebrew link): still found.
        #[cfg(unix)]
        {
            let link = base.path().join("zajil");
            std::os::unix::fs::symlink(&exe, &link).unwrap();
            assert_eq!(host_beside(&link), Ok(found.clone()));
        }
        // An AppImage's mount: refused.
        let mount = base.path().join(".mount_ZajilAb12/usr/bin");
        std::fs::create_dir_all(&mount).unwrap();
        std::fs::write(mount.join("zajil-desktop"), "app").unwrap();
        assert_eq!(
            host_beside(&mount.join("zajil-desktop")),
            Err(Problem::Temporary)
        );

        for problem in [Problem::Missing, Problem::Temporary, Problem::Unsafe] {
            let integration =
                integrate_with(Err(problem), || -> std::io::Result<SystemRegistrar> {
                    panic!("no registrar without a host")
                });
            assert_eq!(integration.host_problem, Some(problem));
            assert!(integration.browsers.is_empty());
            assert_eq!(integration_problems(&integration).len(), 1);
        }
    }

    /// A clipboard for tests: what each look finds, in turn (`None`: no
    /// change since the last look).
    struct FakeClipboard(
        std::collections::VecDeque<std::io::Result<Option<zajil_core::clipboard::Content>>>,
        Option<zajil_core::clipboard::Content>,
    );

    impl zajil_platform::clipboard::Clipboard for FakeClipboard {
        fn changed(&mut self) -> bool {
            match self.0.front() {
                Some(Ok(None)) => {
                    self.0.pop_front();
                    false
                }
                _ => true,
            }
        }

        fn text(&mut self, _max: usize) -> std::io::Result<zajil_core::clipboard::Content> {
            match self.0.pop_front() {
                Some(Ok(Some(content))) => {
                    self.1 = Some(content.clone());
                    Ok(content)
                }
                Some(Err(err)) => Err(err),
                _ => Ok(self
                    .1
                    .clone()
                    .unwrap_or(zajil_core::clipboard::Content::Empty)),
            }
        }
    }

    /// M5 clipboard monitoring: a copied download link is offered once;
    /// what was there when monitoring started, other text, text too long
    /// and links to pages are not; an unchanged clipboard is not read; two
    /// links closer than the gap give one offer; off, the clipboard is not
    /// even opened, and turned on again what is there is not offered; a
    /// read that fails is tried again.
    #[test]
    fn clipboard_monitoring_offers_each_copied_download_link_once() {
        use zajil_core::clipboard::Content;
        let data = tempfile::tempdir().unwrap();
        let categories = CategoryStore::new(data.path());
        let a = "https://example.com/a.zip";
        let b = "https://example.com/b.mp4";
        let t = |s: &str| Ok(Some(Content::Text(s.to_owned())));
        let start = std::time::Instant::now();
        let at = |seconds: u64| start + std::time::Duration::from_secs(seconds * 3);
        let mut watcher = ClipboardWatcher::default();
        let mut opened = 0;
        let mut open = || {
            opened += 1;
            Ok(FakeClipboard(
                [
                    t(a),
                    Ok(None),
                    t("https://example.com/page.html"),
                    t("hello"),
                    Ok(Some(Content::Longer)),
                    t(b),
                    Ok(None),
                    Ok(Some(Content::Empty)),
                    t(a),
                ]
                .into(),
                None,
            ))
        };
        let mut seen = Vec::new();
        for i in 0..9 {
            seen.push(watcher.tick(true, &mut open, &categories, at(i)));
        }
        assert_eq!(opened, 1);
        assert_eq!(
            seen,
            [
                None,
                None,
                None,
                None,
                None,
                Some(b.to_owned()),
                None,
                None,
                Some(a.to_owned())
            ]
        );

        // Two links one second apart: the second is not offered.
        let mut watcher = ClipboardWatcher::default();
        let reads = FakeClipboard([t("x"), t(a), t(b)].into(), None);
        watcher.clipboard = Some(reads);
        let never = || -> std::io::Result<FakeClipboard> { unreachable!() };
        assert_eq!(watcher.tick(true, never, &categories, start), None);
        assert_eq!(
            watcher.tick(true, never, &categories, start).as_deref(),
            Some(a)
        );
        let soon = start + std::time::Duration::from_secs(1);
        assert_eq!(watcher.tick(true, never, &categories, soon), None);

        // Off: not opened, nothing offered; on again, what is there is
        // remembered only.
        let never = || -> std::io::Result<FakeClipboard> { panic!("opened while off") };
        assert_eq!(watcher.tick(false, never, &categories, start), None);
        assert!(watcher.clipboard.is_none());
        let mut watcher = ClipboardWatcher::default();
        let reads = || Ok(FakeClipboard([t(b)].into(), None));
        assert_eq!(watcher.tick(true, reads, &categories, at(0)), None);
        assert_eq!(
            watcher.tick(true, || unreachable!(), &categories, at(1)),
            None
        );

        // A clipboard that cannot be opened, then one whose read fails:
        // both tried again, and the link copied meanwhile is offered.
        let mut watcher = ClipboardWatcher::default();
        let busy = || std::io::Error::other("the clipboard is held by another program");
        assert_eq!(watcher.tick(true, || Err(busy()), &categories, at(0)), None);
        let empty = || Ok(FakeClipboard([Ok(Some(Content::Empty))].into(), None));
        assert_eq!(watcher.tick(true, empty, &categories, at(1)), None);
        watcher.clipboard = Some(FakeClipboard([Err(busy())].into(), None));
        assert_eq!(
            watcher.tick(true, || unreachable!(), &categories, at(2)),
            None
        );
        assert!(watcher.clipboard.is_none(), "closed to be opened again");
        let link = || Ok(FakeClipboard([t(a)].into(), None));
        assert_eq!(
            watcher.tick(true, link, &categories, at(3)).as_deref(),
            Some(a)
        );
    }

    /// M5, end to end on the real channel: a download sent as `zajil-host`
    /// sends it reaches the app's loop (`zajil_host::serve`) and lands in
    /// the queues file, from where the queue runner downloads it.
    #[test]
    fn a_download_through_the_real_channel_lands_in_the_queue() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let listener = zajil_platform::ipc::Listener::bind(data.path()).unwrap();
        let (served, given) = (store.clone(), data.path().to_path_buf());
        std::thread::spawn(move || {
            zajil_host::serve(listener, move |request| {
                from_extension(&served, &request, &given, &Default::default(), |_, _| {
                    Ok(probe_of("b.iso", None))
                })
            });
        });
        let mut stream =
            zajil_platform::ipc::connect(data.path(), std::time::Duration::from_secs(5)).unwrap();
        zajil_host::write_frame(
            &mut stream,
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/b.iso"}"#,
        )
        .unwrap();
        let answer = zajil_host::read_frame(&mut stream, zajil_host::MAX_RESPONSE)
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<zajil_host::Response>(&answer).unwrap(),
            zajil_host::Response::ok()
        );
        let queues = store.load().unwrap();
        assert_eq!(queues.queues[0].items[0].url, "https://example.com/b.iso");
    }

    /// M5: a download from the extension lands in the default queue, by
    /// category, started; a ping changes nothing.
    #[test]
    fn a_download_from_the_extension_is_added_to_the_queue() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let request = zajil_host::parse(
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/a.zip","cookies":"sid=1"}"#,
        )
        .unwrap();
        assert_eq!(
            from_extension(
                &store,
                &request,
                data.path(),
                &Default::default(),
                |_, _| Ok(probe_of("a.zip", None))
            ),
            zajil_host::Response::ok()
        );
        let queues = store.load().unwrap();
        let item = queues.queues[0].items.first().expect("added");
        assert_eq!(queues.queues[0].name, DEFAULT_QUEUE);
        assert_eq!(item.url, "https://example.com/a.zip");
        assert!(item.categorize);
        assert_eq!(item.status, Status::Waiting);
        let ping = zajil_host::parse(br#"{"type":"ping","protocol_version":1}"#).unwrap();
        assert_eq!(
            from_extension(
                &store,
                &ping,
                data.path(),
                &Default::default(),
                |_, _| panic!("a ping reaches nothing")
            ),
            zajil_host::Response::ok()
        );
        assert_eq!(store.load().unwrap().queues[0].items.len(), 1);
    }

    /// What the engine's probe would say: a file `name` of `content_type`.
    fn probe_of(name: &str, content_type: Option<&str>) -> zajil_core::Probe {
        zajil_core::Probe {
            name: name.to_owned(),
            size: Some(10),
            resumable: true,
            ranges: true,
            host: Some("example.com".into()),
            content_type: content_type.map(str::to_owned),
        }
    }

    /// A link that leads to a sign-in page (a redirect answered 200 with a
    /// web page), or to a file of another type, is refused and nothing is
    /// added: the browser keeps its download (review of #70).
    #[test]
    fn a_sign_in_page_or_another_file_is_refused() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let request = zajil_host::parse(
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/l.zip","filename":"l.zip"}"#,
        )
        .unwrap();
        for (probe, why) in [
            (probe_of("signin", Some("text/html")), "a web page"),
            (
                probe_of("l.zip", Some("application/xhtml+xml")),
                "a web page",
            ),
            (
                probe_of("signin.php", Some("application/octet-stream")),
                "another file (signin.php), not a .zip",
            ),
            (
                probe_of("download", None),
                "another file (download), not a .zip",
            ),
        ] {
            let answer = from_extension(
                &store,
                &request,
                data.path(),
                &Default::default(),
                |_, _| Ok(probe),
            );
            assert!(!answer.ok);
            assert!(answer.error.as_deref().unwrap().contains(why), "{answer:?}");
        }
        assert!(store.load().unwrap().queues[0].items.is_empty());
        // The same file, its extension in capitals: taken.
        let answer = from_extension(
            &store,
            &request,
            data.path(),
            &Default::default(),
            |_, _| Ok(probe_of("L.ZIP", Some("application/zip"))),
        );
        assert!(answer.ok, "{answer:?}");
        // No name from the browser: only a web page is refused.
        let bare = zajil_host::parse(
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/get?id=1"}"#,
        )
        .unwrap();
        let answer = from_extension(&store, &bare, data.path(), &Default::default(), |_, _| {
            Ok(probe_of("get", None))
        });
        assert!(answer.ok, "{answer:?}");
    }

    /// M5: the cookies and the page the extension sends go with Zajil's
    /// check, and are kept in memory for the download, under its item; a
    /// download sent without them keeps none.
    #[test]
    fn the_extension_s_cookies_go_with_the_check_and_the_download() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let headers = zajil_core::queue::FromBrowserById::default();
        let request = zajil_host::parse(
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/a.zip","referrer":"https://example.com/page","cookies":"sid=1"}"#,
        )
        .unwrap();
        let mut checked_with = None;
        let answer = from_extension(&store, &request, data.path(), &headers, |_, from| {
            checked_with = Some(from.has_cookies());
            Ok(probe_of("a.zip", None))
        });
        assert!(answer.ok, "{answer:?}");
        assert_eq!(checked_with, Some(true));
        let id = store.load().unwrap().queues[0].items[0].id;
        let kept = headers.lock().unwrap();
        assert!(kept.get(&id).is_some_and(|f| f.has_cookies()));
        drop(kept);
        let bare = zajil_host::parse(
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/b.zip"}"#,
        )
        .unwrap();
        let answer = from_extension(&store, &bare, data.path(), &headers, |_, from| {
            assert!(!from.has_cookies());
            Ok(probe_of("b.zip", None))
        });
        assert!(answer.ok);
        let second = store.load().unwrap().queues[0].items[1].id;
        assert!(!headers.lock().unwrap()[&second].has_cookies());
    }

    /// A queue download that failed when its link expired, with part of
    /// the file on disk.
    async fn expired_download(
        server: &zajil_testserver::TestServer,
        data: &[u8],
        base: &Path,
    ) -> (Engine, QueueStore, u64) {
        use zajil_testserver::FileSpec;
        let spec = || FileSpec::new(data.to_vec()).etag(Some("\"v1\""));
        server.put("/token1/f.bin", spec().cut_after(256 * 1024, 1));
        let engine = Engine::new(Options {
            connections: 1,
            max_failures: 1,
            ..Options::default()
        })
        .unwrap();
        let store = QueueStore::new(base);
        let url = server.url("/token1/f.bin");
        let dir = base.join("out");
        let id = store.update(|q| q.add(DEFAULT_QUEUE, &url, &dir)).unwrap();
        Runner::new(engine.clone(), store.clone())
            .poll(Duration::from_millis(20))
            .run(&Stop::new(), &mut |_| {})
            .await
            .unwrap();
        let item = store.load().unwrap().item(id).unwrap().clone();
        assert_eq!(item.status, Status::Failed, "{item:?}");
        assert!(item.state_file.as_ref().is_some_and(|p| p.exists()));
        // The signed link expired.
        server.put("/token1/f.bin", FileSpec::new(Vec::new()).forbid_after(0));
        (engine, store, id)
    }

    /// The ROADMAP criterion in the app: a download whose link expired
    /// goes on from a new link to the same file, asking only for what is
    /// missing, and its item keeps the new link.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_download_goes_on_from_its_refreshed_link() {
        use zajil_testserver::{FileSpec, TestServer, random_bytes};
        let server = TestServer::http();
        let data = random_bytes(1024 * 1024, 94);
        let base = tempfile::tempdir().unwrap();
        let (engine, store, id) = expired_download(&server, &data, base.path()).await;
        server.put(
            "/token2/f.bin",
            FileSpec::new(data.clone()).etag(Some("\"v1\"")),
        );
        let headers = zajil_core::queue::FromBrowserById::default();
        let refreshing = Refreshing::default();
        // As a person may type it: kept as the parser writes it.
        let typed = server.url("/token2/f.bin").replace("http://", "HTTP://");
        refresh_item(&engine, &store, &headers, &refreshing, id, &typed)
            .await
            .unwrap();
        let item = store.load().unwrap().item(id).unwrap().clone();
        assert_eq!(item.url, server.url("/token2/f.bin"));
        assert_eq!((item.status, item.error), (Status::Waiting, None));
        Runner::new(engine, store.clone())
            .poll(Duration::from_millis(20))
            .run(&Stop::new(), &mut |_| {})
            .await
            .unwrap();
        let item = store.load().unwrap().item(id).unwrap().clone();
        assert_eq!(item.status, Status::Done, "{item:?}");
        assert_eq!(std::fs::read(item.file.unwrap()).unwrap(), data);
        let gets = server.gets("/token2/f.bin");
        assert_eq!(gets.len(), 2, "the check, then the rest: {gets:?}");
        let range = gets[1].range.as_deref().unwrap();
        assert!(!range.starts_with("bytes=0-"), "started over: {range}");
    }

    /// A link to another file is refused with the reason, and the download
    /// is left as it was; and what may not be refreshed is not.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_refresh_is_refused_for_another_file_and_while_it_may_not_be() {
        use view::MismatchView;
        use zajil_testserver::{FileSpec, TestServer, random_bytes};
        let server = TestServer::http();
        let data = random_bytes(1024 * 1024, 95);
        let base = tempfile::tempdir().unwrap();
        let (engine, store, id) = expired_download(&server, &data, base.path()).await;
        let before = store.load().unwrap().item(id).unwrap().clone();
        let state_before = std::fs::read(before.state_file.as_ref().unwrap()).unwrap();
        server.put(
            "/other.bin",
            FileSpec::new(random_bytes(1000, 96)).etag(Some("\"v1\"")),
        );
        let headers = zajil_core::queue::FromBrowserById::default();
        let refreshing = Refreshing::default();
        let refresh = |url: String| {
            let (engine, store, headers, refreshing) = (&engine, &store, &headers, &refreshing);
            async move { refresh_item(engine, store, headers, refreshing, id, &url).await }
        };
        assert_eq!(
            refresh(server.url("/other.bin")).await,
            Err(RefreshProblem::NotTheSameFile {
                why: MismatchView::Size,
                theirs: Some(1000),
                ours: Some(data.len() as u64),
            })
        );
        assert!(matches!(
            refresh("https://user:pw@example.com/f.bin".into()).await,
            Err(RefreshProblem::BadLink { .. })
        ));
        assert_eq!(store.load().unwrap().item(id).unwrap(), &before);
        assert_eq!(
            std::fs::read(before.state_file.as_ref().unwrap()).unwrap(),
            state_before
        );
        // Another refresh of it under way: refused, and it is not resumed.
        let held = refreshing.start(id).unwrap();
        assert_eq!(
            refresh(server.url("/other.bin")).await,
            Err(RefreshProblem::NotStopped)
        );
        assert_eq!(refreshing.without(vec![id, id + 1]), vec![id + 1]);
        drop(held);
        assert_eq!(refreshing.without(vec![id]), vec![id]);
        for (status, problem) in [
            (Status::Waiting, RefreshProblem::NotStopped),
            (Status::Active, RefreshProblem::NotStopped),
            (Status::Done, RefreshProblem::Done),
        ] {
            store
                .update(|q| {
                    q.item_mut(id).unwrap().status = status;
                    Ok(())
                })
                .unwrap();
            assert_eq!(refresh(server.url("/other.bin")).await, Err(problem));
        }
    }

    /// A download with nothing on disk yet just takes the new link.
    #[tokio::test]
    async fn a_download_not_started_takes_the_new_link() {
        let base = tempfile::tempdir().unwrap();
        let store = QueueStore::new(base.path());
        let dir = base.path().join("out");
        let id = store
            .update(|q| q.add(DEFAULT_QUEUE, "https://example.com/old.iso", &dir))
            .unwrap();
        store
            .update(|q| {
                q.item_mut(id).unwrap().status = Status::Paused;
                Ok(())
            })
            .unwrap();
        let engine = Engine::new(Options::default()).unwrap();
        refresh_item(
            &engine,
            &store,
            &Default::default(),
            &Refreshing::default(),
            id,
            "https://example.com/new.iso",
        )
        .await
        .unwrap();
        let item = store.load().unwrap().item(id).unwrap().clone();
        assert_eq!(item.url, "https://example.com/new.iso");
        assert_eq!(item.status, Status::Waiting);
    }

    /// The glue in the app (review of #76): the check of a download from
    /// the extension sends its cookies and page, and the window's runner
    /// sends them with the download.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_check_and_the_window_s_runner_send_the_browser_s_headers() {
        use zajil_testserver::{FileSpec, TestServer, random_bytes};
        let server = TestServer::http();
        let data = random_bytes(300_000, 93);
        server.put("/a.zip", FileSpec::new(data.clone()));
        let url = server.url("/a.zip");
        let page = server.url("/page");
        let from = zajil_core::FromBrowser::new(&url, Some("sid=GLUE"), Some(&page));
        let engine = Engine::new(Options::default()).unwrap();

        let found = tokio::task::spawn_blocking({
            let (url, from, engine) = (url.clone(), from.clone(), engine.clone());
            move || extension_reach(&engine)(&url, &from)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(found.name, "a.zip");
        assert_eq!(server.gets("/a.zip")[0].cookie.as_deref(), Some("sid=GLUE"));

        let base = tempfile::tempdir().unwrap();
        let store = QueueStore::new(base.path());
        let dir = base.path().join("out");
        let id = store.update(|q| q.add(DEFAULT_QUEUE, &url, &dir)).unwrap();
        let headers = zajil_core::queue::FromBrowserById::default();
        headers.lock().unwrap().insert(id, from);
        let runner = queue_runner(
            &engine,
            &store,
            &CategoryStore::new(base.path()),
            &Arc::new(AtomicUsize::new(1)),
            &headers,
        );
        runner
            .poll(Duration::from_millis(20))
            .run(&Stop::new(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(std::fs::read(dir.join("a.zip")).unwrap(), data);
        for get in server.gets("/a.zip").iter().skip(1) {
            assert_eq!(get.cookie.as_deref(), Some("sid=GLUE"), "{get:?}");
            assert_eq!(get.referer.as_deref(), Some(page.as_str()));
        }
    }

    /// A file Zajil cannot reach itself (behind a login, a referrer check,
    /// or gone) is refused and nothing is added, so the extension leaves it
    /// to the browser (review of #70).
    #[test]
    fn a_file_zajil_cannot_reach_is_refused_and_not_added() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let request = zajil_host::parse(
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/private.zip"}"#,
        )
        .unwrap();
        let mut asked = Vec::new();
        let answer = from_extension(
            &store,
            &request,
            data.path(),
            &Default::default(),
            |url, _| {
                asked.push(url.to_owned());
                Err("the server answered 403 Forbidden".into())
            },
        );
        assert!(!answer.ok);
        assert_eq!(
            answer.error.as_deref(),
            Some("Zajil cannot download it: the server answered 403 Forbidden")
        );
        assert_eq!(asked, ["https://example.com/private.zip"]);
        assert!(store.load().unwrap().queues[0].items.is_empty());
    }

    /// A folder that comes from the page is checked before anything is
    /// made there; without one, the download folder is used.
    #[test]
    fn a_folder_from_the_page_is_checked() {
        let base = std::env::temp_dir();
        assert_eq!(chosen_folder(None, &base).unwrap(), base);
        let own = base.join("zajil-own");
        assert_eq!(chosen_folder(own.to_str(), &base).unwrap(), own);
        for bad in ["relative/dir", "../up"] {
            assert!(chosen_folder(Some(bad), &base).is_err(), "{bad}");
        }
        let dotdot = base.join("..").join("x");
        assert!(chosen_folder(dotdot.to_str(), &base).is_err());
        let hidden = format!("{}\u{202e}x", base.display());
        assert!(chosen_folder(Some(&hidden), &base).is_err());
    }

    /// The owner's condition on #47: in a release build the commands of
    /// the UI tests do not exist, and calling them is an error; in a
    /// `ui-test` build the same calls answer (CI runs both).
    #[test]
    fn the_ui_test_commands_exist_only_in_a_ui_test_build() {
        let app = tauri::test::mock_builder()
            .invoke_handler(handlers())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        #[cfg(feature = "ui-test")]
        app.manage(ui_test::UiTest::from_args(&[]));
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        for cmd in ["ui_test_scenario", "ui_test_report"] {
            let body = serde_json::json!({ "report": {} });
            let answer = tauri::test::get_ipc_response(
                &webview,
                tauri::webview::InvokeRequest {
                    cmd: cmd.into(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    // The page's own origin: anything else is refused before
                    // the command is even looked for.
                    url: if cfg!(windows) {
                        "http://tauri.localhost"
                    } else {
                        "tauri://localhost"
                    }
                    .parse()
                    .unwrap(),
                    body: tauri::ipc::InvokeBody::Json(body),
                    headers: Default::default(),
                    invoke_key: tauri::test::INVOKE_KEY.to_string(),
                },
            );
            if cfg!(feature = "ui-test") {
                assert!(answer.is_ok(), "{cmd}: {answer:?}");
            } else {
                let error = answer.expect_err(cmd).to_string();
                assert!(error.contains("not found"), "{cmd}: {error}");
            }
        }
    }

    #[test]
    fn a_damaged_categories_file_is_said_and_the_downloads_still_show() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        store
            .update(|q| q.add("main", "https://example.com/a.zip", data.path()))
            .unwrap();
        let categories = CategoryStore::new(data.path());
        std::fs::write(categories.path(), b"{ damaged").unwrap();
        let snapshot = read_snapshot(&store, &categories, &Live::default()).unwrap();
        assert_eq!(snapshot.downloads.len(), 1);
        assert!(snapshot.categories.is_empty());
        let problem = snapshot.problem.expect("the problem is said");
        assert!(problem.contains("categories"), "{problem}");
    }

    #[test]
    fn an_unknown_id_is_an_error_and_changes_nothing() {
        let mut q = three();
        let before = q.clone();
        assert!(matches!(
            set_status(&mut q, &[1, 99], Status::Paused),
            Err(StoreError::NoSuchItem(99))
        ));
        assert!(matches!(
            remove_items(&mut q, &[2, 99]),
            Err(StoreError::NoSuchItem(99))
        ));
        assert_eq!(q, before);
        remove_items(&mut q, &[2]).unwrap();
        assert!(q.item(2).is_none());
        assert!(q.item(1).is_some());
    }
}

#[tauri::command]
async fn details<R: tauri::Runtime>(
    app: State<'_, App>,
    id: u64,
    handle: tauri::AppHandle<R>,
) -> Result<DetailsView, String> {
    // The demo download's details only in a scene (review of #47).
    #[cfg(feature = "ui-test")]
    if handle.state::<ui_test::UiTest>().scenario.is_some() {
        return ui_test::demo_details(id).ok_or_else(|| format!("no download {id}"));
    }
    #[cfg(not(feature = "ui-test"))]
    let _ = handle;
    let (store, categories, live) = (app.store.clone(), app.categories.clone(), app.live.clone());
    tokio::task::spawn_blocking(move || {
        let queues = store.load().map_err(blocking_err)?;
        let (queue, item) = queues
            .queues
            .iter()
            .find_map(|q| {
                q.items
                    .iter()
                    .find(|i| i.id == id)
                    .map(|i| (q.name.as_str(), i))
            })
            .ok_or_else(|| format!("no download {id}"))?;
        // A damaged categories file: none, as in the list (which says the
        // problem itself).
        let categories = categories.load().unwrap_or_else(|_| no_categories());
        let state = item
            .state_file
            .as_deref()
            .and_then(|p| DownloadState::load(p).ok());
        Ok(view::details(
            item,
            queue,
            &categories,
            state.as_ref(),
            &live,
        ))
    })
    .await
    .map_err(blocking_err)?
}

/// The download folder: the one in the settings, or the system's. Blocking.
fn download_dir(settings: &SettingsStore, system: &Path) -> PathBuf {
    settings
        .load()
        .ok()
        .and_then(|s| s.download_dir)
        .unwrap_or_else(|| system.to_path_buf())
}

#[tauri::command]
async fn settings(app: State<'_, App>) -> Result<SettingsView, String> {
    let (settings, store, system) = (
        app.settings.clone(),
        app.store.clone(),
        app.system_download_dir.clone(),
    );
    tokio::task::spawn_blocking(move || {
        let file = settings.load().map_err(blocking_err)?;
        let limit = store.load().map_err(blocking_err)?.settings.speed_limit;
        Ok(view::settings(&file, limit, &system))
    })
    .await
    .map_err(blocking_err)?
}

/// Saves the settings screen: `settings.json`, and the speed limit in the
/// queues file, where the runner reads it within half a second. The next
/// download takes the new number of connections.
/// A `ui-test` scene shows demo data: nothing done in it may change the
/// user's files (review of #47).
fn in_scene<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) -> bool {
    #[cfg(feature = "ui-test")]
    {
        handle.state::<ui_test::UiTest>().scenario.is_some()
    }
    #[cfg(not(feature = "ui-test"))]
    {
        let _ = handle;
        false
    }
}

#[tauri::command]
async fn save_settings<R: tauri::Runtime>(
    app: State<'_, App>,
    change: SettingsChange,
    handle: tauri::AppHandle<R>,
) -> Result<SettingsView, String> {
    if in_scene(&handle) {
        // The language-switch scene goes through here: applied, not saved.
        let mut file = app.settings.load().unwrap_or_default();
        view::apply_settings(&mut file, &change);
        return Ok(view::settings(
            &file,
            change.speed_limit,
            &app.system_download_dir,
        ));
    }
    let (settings, store, system, connections, clipboard) = (
        app.settings.clone(),
        app.store.clone(),
        app.system_download_dir.clone(),
        app.connections.clone(),
        app.clipboard.clone(),
    );
    let saved = tokio::task::spawn_blocking(move || {
        if change.speed_limit == Some(0) {
            return Err("a speed limit must be more than 0".to_owned());
        }
        let file = settings
            .update(|file| {
                view::apply_settings(file, &change);
                Ok(file.clone())
            })
            .map_err(blocking_err)?;
        store
            .update(|queues| {
                queues.settings.speed_limit = change.speed_limit;
                Ok(())
            })
            .map_err(blocking_err)?;
        connections.store(file.connections as usize, Ordering::Relaxed);
        clipboard.store(file.clipboard, Ordering::Relaxed);
        Ok(view::settings(&file, change.speed_limit, &system))
    })
    .await
    .map_err(blocking_err)??;
    let _ = handle.emit("downloads-changed", ());
    Ok(saved)
}

#[tauri::command]
async fn categories(app: State<'_, App>) -> Result<Vec<CategoryView>, String> {
    let categories = app.categories.clone();
    tokio::task::spawn_blocking(move || {
        Ok(view::category_views(
            &categories.load().map_err(blocking_err)?,
        ))
    })
    .await
    .map_err(blocking_err)?
}

/// Replaces the categories with the page's list, checked in full: nothing
/// is written if any value is invalid.
#[tauri::command]
async fn save_categories<R: tauri::Runtime>(
    app: State<'_, App>,
    categories: Vec<CategoryView>,
    handle: tauri::AppHandle<R>,
) -> Result<Vec<CategoryView>, String> {
    if in_scene(&handle) {
        return Err("a ui-test scene saves nothing".into());
    }
    let store = app.categories.clone();
    let saved = tokio::task::spawn_blocking(move || {
        store
            .update(|file| {
                file.categories = view::categories_from(&categories);
                Ok(view::category_views(file))
            })
            .map_err(blocking_err)
    })
    .await
    .map_err(blocking_err)??;
    let _ = handle.emit("downloads-changed", ());
    Ok(saved)
}

/// The category `name` would go to with the page's list, before it is
/// saved ("try a name").
#[tauri::command]
fn try_category(categories: Vec<CategoryView>, name: String) -> Result<Option<String>, String> {
    let file = CategoriesFile {
        categories: view::categories_from(&categories),
        ..CategoriesFile::built_in()
    };
    file.check_consistency()?;
    Ok(file.classify(&name, None).map(|c| c.id.clone()))
}

/// Writes the queues to a new file the user chose (never over a file).
#[tauri::command]
async fn export_queues(app: State<'_, App>, path: String) -> Result<(), String> {
    let store = app.store.clone();
    tokio::task::spawn_blocking(move || {
        let queues = store.load().map_err(blocking_err)?;
        zajil_core::queue::write_export(Path::new(&path), &queues.export()).map_err(blocking_err)
    })
    .await
    .map_err(blocking_err)?
}

/// Imports a file the user chose, checked in full before anything changes.
/// Its folders are used only if the user asked; otherwise the downloads go
/// to the download folder, by category.
#[tauri::command]
async fn import_queues<R: tauri::Runtime>(
    app: State<'_, App>,
    path: String,
    keep_folders: bool,
    handle: tauri::AppHandle<R>,
) -> Result<ImportView, String> {
    if in_scene(&handle) {
        return Err("a ui-test scene imports nothing".into());
    }
    let (store, settings, system) = (
        app.store.clone(),
        app.settings.clone(),
        app.system_download_dir.clone(),
    );
    let done = tokio::task::spawn_blocking(move || {
        let export = zajil_core::queue::read_export(Path::new(&path)).map_err(blocking_err)?;
        let folders = if keep_folders {
            zajil_core::queue::Folders::Keep
        } else {
            zajil_core::queue::Folders::Into(download_dir(&settings, &system))
        };
        store
            .update(|queues| queues.import(&export, &folders))
            .map(|summary| ImportView::from(&summary))
            .map_err(blocking_err)
    })
    .await
    .map_err(blocking_err)??;
    let _ = handle.emit("downloads-changed", ());
    Ok(done)
}
