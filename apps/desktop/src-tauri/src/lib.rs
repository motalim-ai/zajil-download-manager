//! Zajil's desktop interface (M4, ADR-0003, ADR-0008): a Tauri 2 window
//! over the engine. Like the command line, it holds no download logic
//! (ARCHITECTURE §1): it runs the queues with `zajil_core::queue::Runner`,
//! sends the user's commands to the queues file, and shows what happens.
//!
//! The page asks for a [`view::Snapshot`] when told that something changed
//! (event `downloads-changed`, at most four times a second).

#![forbid(unsafe_code)]

mod live;
pub mod view;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager, State};
use zajil_core::category::{CategoriesFile, CategoryStore};
use zajil_core::queue::{QueueEvent, QueueStore, QueuesFile, Runner, Status, StoreError};
use zajil_core::{Engine, Options, Stop};
use zajil_platform::{AppPaths, SystemPaths};

use crate::live::{Live, NOTIFY_EVERY, Notify};
use crate::view::Snapshot;

/// How long the program waits for the downloads to record their progress
/// when it is closed.
const STOP_WAIT: Duration = Duration::from_secs(5);

/// Everything the commands share.
pub struct App {
    engine: Engine,
    store: QueueStore,
    categories: CategoryStore,
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
    let app = App {
        engine: Engine::new(Options::default()).expect("the engine starts"),
        store: QueueStore::new(&data),
        categories: CategoryStore::new(&data),
        live: Arc::new(Live::default()),
        stop: Stop::new(),
        stopped: tokio::sync::watch::Sender::new(false),
        exiting: AtomicBool::new(false),
        page_ready: AtomicBool::new(false),
    };
    #[cfg(unix)]
    let signals = stop_signals();
    tauri::Builder::default()
        .manage(app)
        .invoke_handler(tauri::generate_handler![snapshot, pause, resume, remove])
        .setup(move |tauri_app| {
            let handle = tauri_app.handle().clone();
            #[cfg(unix)]
            if let Some(signals) = signals {
                tauri::async_runtime::spawn(exit_on_signal(handle.clone(), signals));
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
async fn snapshot(app: State<'_, App>) -> Result<Snapshot, String> {
    if !app.page_ready.swap(true, Ordering::SeqCst) {
        // The page is up and talking to the program (the open-and-close
        // check of CI waits for this line).
        eprintln!("zajil: the window is ready");
    }
    let (store, categories, live) = (app.store.clone(), app.categories.clone(), app.live.clone());
    tokio::task::spawn_blocking(move || read_snapshot(&store, &categories, &live))
        .await
        .map_err(blocking_err)?
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
