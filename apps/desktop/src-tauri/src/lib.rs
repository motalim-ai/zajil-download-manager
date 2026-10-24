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
use zajil_platform::{AppPaths, SystemPaths};

use crate::live::{Live, NOTIFY_EVERY, Notify};
use crate::view::{
    AddRequest, CategoryView, DetailsView, ImportView, ProbeView, SettingsChange, SettingsView,
    Snapshot,
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
    let connections = settings
        .load()
        .map(|s| s.connections as usize)
        .unwrap_or(zajil_core::DEFAULT_CONNECTIONS);
    let app = App {
        engine: Engine::new(Options::default()).expect("the engine starts"),
        store: QueueStore::new(&data),
        categories: CategoryStore::new(&data),
        settings,
        system_download_dir,
        connections: Arc::new(AtomicUsize::new(connections)),
        live: Arc::new(Live::default()),
        stop: Stop::new(),
        stopped: tokio::sync::watch::Sender::new(false),
        exiting: AtomicBool::new(false),
        page_ready: AtomicBool::new(false),
        probing: std::sync::Mutex::new(None),
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
        import_queues
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
    let (engine, store, categories, connections, live, stop) = {
        let app = handle.state::<App>();
        (
            app.engine.clone(),
            app.store.clone(),
            app.categories.clone(),
            app.connections.clone(),
            app.live.clone(),
            app.stop.clone(),
        )
    };
    loop {
        if stop.is_requested() {
            return;
        }
        let runner = Runner::new(engine.clone(), store.clone())
            .categories(categories.clone())
            .connections(connections.clone());
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
/// user's private channel (ADR-0005, M5). One thread waits for
/// connections; each carries one request and its answer. A second app on
/// the same data folder cannot open the channel: it says so and goes on
/// without it.
fn serve_extension<R: tauri::Runtime>(handle: tauri::AppHandle<R>, data: PathBuf) {
    let listener = match zajil_platform::ipc::Listener::bind(&data) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("zajil: the browser extension cannot reach this window: {err}");
            return;
        }
    };
    std::thread::spawn(move || {
        while let Ok(mut stream) = listener.accept() {
            let handle = handle.clone();
            std::thread::spawn(move || {
                let Ok(Some(message)) =
                    zajil_host::read_frame(&mut stream, zajil_host::MAX_REQUEST)
                else {
                    return;
                };
                let answer = match zajil_host::parse(&message) {
                    Err(why) => zajil_host::Response::refused(why),
                    Ok(request) if in_scene(&handle) => {
                        let _ = request;
                        zajil_host::Response::refused("a ui-test scene adds nothing")
                    }
                    Ok(request) => {
                        let app = handle.state::<App>();
                        let given = download_dir(&app.settings, &app.system_download_dir);
                        let answer = from_extension(&app.store, &request, &given);
                        if answer.ok {
                            let _ = handle.emit("downloads-changed", ());
                            tray::show_window(&handle);
                        }
                        answer
                    }
                };
                let _ = zajil_host::write_json(&mut stream, &answer);
            });
        }
    });
}

/// What a request from the extension does: a download goes into the
/// default queue, by category, as the add dialog adds one. The referrer,
/// the suggested name and the cookies are checked by `zajil_host::parse`
/// and carried, but not used yet: the engine sends no such headers until
/// a later part of M5.
fn from_extension(
    store: &QueueStore,
    request: &zajil_host::Request,
    given: &Path,
) -> zajil_host::Response {
    match request {
        zajil_host::Request::Ping { .. } => zajil_host::Response::ok(),
        zajil_host::Request::Download { url, .. } => {
            let add = AddRequest {
                url: url.clone(),
                queue: None,
                folder: None,
                start: true,
                limit: None,
            };
            match add_blocking(store, &add, given) {
                Ok(_) => zajil_host::Response::ok(),
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
    change(&app, &handle, move |q| set_status(q, &ids, Status::Waiting)).await
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
            from_extension(&store, &request, data.path()),
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
            from_extension(&store, &ping, data.path()),
            zajil_host::Response::ok()
        );
        assert_eq!(store.load().unwrap().queues[0].items.len(), 1);
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
    let (settings, store, system, connections) = (
        app.settings.clone(),
        app.store.clone(),
        app.system_download_dir.clone(),
        app.connections.clone(),
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
