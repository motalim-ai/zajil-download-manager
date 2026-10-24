//! Zajil's desktop interface (M4, ADR-0003, ADR-0008): a Tauri 2 window
//! over the engine. Like the command line, it holds no download logic
//! (ARCHITECTURE §1): it runs the queues with `zajil_core::queue::Runner`,
//! sends the user's commands to the queues file, and shows what happens.
//!
//! The page asks for a [`view::Snapshot`] when told that something changed
//! (event `downloads-changed`, at most four times a second).

#![forbid(unsafe_code)]

mod live;
#[cfg(feature = "ui-test")]
mod ui_test;
pub mod view;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager, State};
use zajil_core::category::{CategoriesFile, CategoryStore};
use zajil_core::queue::{
    DEFAULT_QUEUE, QueueEvent, QueueStore, QueuesFile, Runner, Status, StoreError,
};
use zajil_core::state::DownloadState;
use zajil_core::{Engine, Options, Stop};
use zajil_platform::{AppPaths, SystemPaths};

use crate::live::{Live, NOTIFY_EVERY, Notify};
use crate::view::{AddRequest, DetailsView, ProbeView, Snapshot};

/// How long the program waits for the downloads to record their progress
/// when it is closed.
const STOP_WAIT: Duration = Duration::from_secs(5);

/// Everything the commands share.
pub struct App {
    engine: Engine,
    store: QueueStore,
    categories: CategoryStore,
    download_dir: PathBuf,
    live: Arc<Live>,
    stop: Stop,
    /// Set once the queues have stopped (their downloads recorded their
    /// progress), so the program may exit.
    stopped: tokio::sync::watch::Sender<bool>,
    /// The first exit request is being handled.
    exiting: AtomicBool,
    /// The page has asked for its first snapshot.
    page_ready: AtomicBool,
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
    let download_dir = SystemPaths::new()
        .map(|p| p.download_dir().to_path_buf())
        .unwrap_or_else(|_| data.join("Downloads"));
    let app = App {
        engine: Engine::new(Options::default()).expect("the engine starts"),
        store: QueueStore::new(&data),
        categories: CategoryStore::new(&data),
        download_dir,
        live: Arc::new(Live::default()),
        stop: Stop::new(),
        stopped: tokio::sync::watch::Sender::new(false),
        exiting: AtomicBool::new(false),
        page_ready: AtomicBool::new(false),
    };
    #[cfg(unix)]
    let signals = stop_signals();
    let builder = tauri::Builder::default().manage(app);
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
            #[cfg(feature = "ui-test")]
            if tauri_app.state::<ui_test::UiTest>().scenario.is_some() {
                // Fixed demo data: nothing runs, so nothing to wait for.
                tauri_app.state::<App>().stopped.send_replace(true);
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
                if *app.stopped.borrow() {
                    return;
                }
                app.exiting.store(true, Ordering::SeqCst);
                app.stop.request();
                tauri::async_runtime::block_on(wait_for_queues(app.stopped.subscribe()));
            }
            _ => {}
        });
}

#[cfg(not(feature = "ui-test"))]
fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![snapshot, probe, add, pause, resume, remove, details]
}

#[cfg(feature = "ui-test")]
fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        snapshot,
        probe,
        add,
        pause,
        resume,
        remove,
        details,
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
/// Windows the system closes the window instead (#49 for a shutdown).
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
/// progress, and with code 0.
#[cfg(unix)]
async fn exit_on_signal(handle: tauri::AppHandle, mut signals: StopSignals) {
    let name = tokio::select! {
        _ = signals.terminate.recv() => "SIGTERM",
        _ = signals.interrupt.recv() => "SIGINT",
        _ = signals.hangup.recv() => "SIGHUP",
    };
    eprintln!("zajil: asked to stop ({name})");
    handle.exit(0);
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
    let (engine, store, categories, live, stop) = {
        let app = handle.state::<App>();
        (
            app.engine.clone(),
            app.store.clone(),
            app.categories.clone(),
            app.live.clone(),
            app.stop.clone(),
        )
    };
    loop {
        if stop.is_requested() {
            return;
        }
        let runner = Runner::new(engine.clone(), store.clone()).categories(categories.clone());
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
                Err(err) => live.set_problem(Some(err)),
            }
        }
    }
}

fn blocking_err(err: impl std::fmt::Display) -> String {
    err.to_string()
}

#[tauri::command]
async fn snapshot(app: State<'_, App>) -> Result<Snapshot, String> {
    if !app.page_ready.swap(true, Ordering::SeqCst) {
        // The page is up and talking to the program (the open-and-close
        // check of CI waits for this line).
        eprintln!("zajil: the window is ready");
    }
    #[cfg(feature = "ui-test")]
    if let Some(snapshot) = app_ui_test_snapshot(&app) {
        return Ok(snapshot);
    }
    let (store, categories, live) = (app.store.clone(), app.categories.clone(), app.live.clone());
    tokio::task::spawn_blocking(move || {
        let queues = store.load().map_err(blocking_err)?;
        // A damaged categories file is said, not fatal: the downloads still
        // show (review of #45: the whole list went blank while they ran).
        let (categories, problem) = match categories.load() {
            Ok(c) => (c, None),
            Err(err) => {
                let mut none = CategoriesFile::built_in();
                none.categories.clear();
                (none, Some(err.to_string()))
            }
        };
        let mut snapshot = view::snapshot(&queues, &categories, &live);
        snapshot.problem = snapshot.problem.or(problem);
        Ok(snapshot)
    })
    .await
    .map_err(blocking_err)?
}

#[cfg(feature = "ui-test")]
fn app_ui_test_snapshot(_app: &App) -> Option<Snapshot> {
    // The fixture does not depend on the real queues.
    Some(ui_test::demo_snapshot())
}

#[tauri::command]
async fn probe(app: State<'_, App>, url: String) -> Result<ProbeView, String> {
    let found = app.engine.probe(&url).await.map_err(|e| e.to_string())?;
    let categories = app.categories.clone();
    let given = app.download_dir.clone();
    tokio::task::spawn_blocking(move || {
        let categories = categories.load().map_err(blocking_err)?;
        Ok(view::probe(&found, &categories, &given))
    })
    .await
    .map_err(blocking_err)?
}

#[tauri::command]
async fn add(
    app: State<'_, App>,
    request: AddRequest,
    handle: tauri::AppHandle,
) -> Result<u64, String> {
    let store = app.store.clone();
    let given = request
        .folder
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| app.download_dir.clone());
    let id = tokio::task::spawn_blocking(move || add_blocking(&store, &request, &given))
        .await
        .map_err(blocking_err)?
        .map_err(|e| e.to_string())?;
    let _ = handle.emit("downloads-changed", ());
    Ok(id)
}

fn add_blocking(store: &QueueStore, request: &AddRequest, given: &Path) -> Result<u64, StoreError> {
    std::fs::create_dir_all(given)?;
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

async fn change(
    app: &App,
    handle: &tauri::AppHandle,
    change: impl FnOnce(&mut QueuesFile) -> Result<(), StoreError> + Send + 'static,
) -> Result<(), String> {
    let store = app.store.clone();
    tokio::task::spawn_blocking(move || store.update(change))
        .await
        .map_err(blocking_err)?
        .map_err(|e| e.to_string())?;
    let _ = handle.emit("downloads-changed", ());
    Ok(())
}

#[tauri::command]
async fn pause(app: State<'_, App>, ids: Vec<u64>, handle: tauri::AppHandle) -> Result<(), String> {
    change(&app, &handle, move |q| set_status(q, &ids, Status::Paused)).await
}

#[tauri::command]
async fn resume(
    app: State<'_, App>,
    ids: Vec<u64>,
    handle: tauri::AppHandle,
) -> Result<(), String> {
    change(&app, &handle, move |q| set_status(q, &ids, Status::Waiting)).await
}

#[tauri::command]
async fn remove(
    app: State<'_, App>,
    ids: Vec<u64>,
    handle: tauri::AppHandle,
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
async fn details(app: State<'_, App>, id: u64) -> Result<DetailsView, String> {
    #[cfg(feature = "ui-test")]
    if app_ui_test_snapshot(&app).is_some() {
        return ui_test::demo_details(id).ok_or_else(|| format!("no download {id}"));
    }
    let (store, live) = (app.store.clone(), app.live.clone());
    tokio::task::spawn_blocking(move || {
        let queues = store.load().map_err(blocking_err)?;
        let item = queues.item(id).ok_or_else(|| format!("no download {id}"))?;
        let state = item
            .state_file
            .as_deref()
            .and_then(|p| DownloadState::load(p).ok());
        Ok(view::details(item, state.as_ref(), &live))
    })
    .await
    .map_err(blocking_err)?
}
