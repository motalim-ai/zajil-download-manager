//! The window's queue runner: it runs the queues while the window is
//! open and tells the page what changed.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager};
use zajil_core::Engine;
use zajil_core::category::CategoryStore;
use zajil_core::queue::{QueueEvent, QueueStore, Runner, Status, StoreError};

use crate::asks::{BrowserAsks, ask_browser, send_keep};
use crate::live::{Live, NOTIFY_EVERY, Notify};
use crate::{App, background, complete, schedule};

/// Tells the page that something changed: at once, or once the throttle
/// allows, so the last change of a burst is never lost.
pub(crate) fn notify(handle: &tauri::AppHandle, live: &Arc<Live>) {
    match live.notify_at(Instant::now()) {
        Notify::Now => tell(handle, live),
        Notify::Later(wait) => {
            let (handle, live) = (handle.clone(), live.clone());
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(wait.min(NOTIFY_EVERY)).await;
                live.notified_later(Instant::now());
                tell(&handle, &live);
            });
        }
        Notify::Scheduled => {}
    }
}

/// The notification itself: the whole list changed, or only the running
/// downloads' progress (then only their rows go: the owner's choice, after
/// measuring the list's cost).
fn tell(handle: &tauri::AppHandle, live: &Live) {
    if live.take_changed() {
        let _ = handle.emit("downloads-changed", ());
    } else {
        crate::downloads::tell_progress(handle);
    }
}

/// Runs the queues for as long as the window is open. The runner ends when
/// nothing is left to download; it starts again as soon as something is.
/// Takes every private window's download out of the queues and deletes the
/// part and state files of those not running (a running one is stopped,
/// and the runner deletes its files): when the last private window closes,
/// and when the window starts, for any left from before (review of #91).
/// Blocking.
pub(crate) fn discard_private(store: &QueueStore) -> Result<(), StoreError> {
    let files = store.update(|q| Ok(q.take_private()))?;
    for file in files {
        let _ = std::fs::remove_file(file);
    }
    Ok(())
}

pub(crate) async fn run_queues(handle: tauri::AppHandle) {
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
    // A private window's download left from before (closed or crashed
    // meanwhile): nothing of it stays.
    {
        let store = store.clone();
        let _ = tokio::task::spawn_blocking(move || discard_private(&store)).await;
    }
    let (links, asks) = {
        let app = handle.state::<App>();
        (app.links.clone(), app.asks.clone())
    };
    loop {
        if stop.is_requested() {
            return;
        }
        // Queues started now, outside their window (#125, the schedule window).
        let run_now = handle.state::<schedule::SharedAfter>().run_now.clone();
        let runner = queue_runner(&engine, &store, &categories, &connections, &from_browser)
            .run_now(run_now);
        let result = runner
            .run(&stop, &mut |event: QueueEvent| {
                let changed = {
                    let handle = handle.clone();
                    move || {
                        let _ = handle.emit("downloads-changed", ());
                    }
                };
                on_queue_event(&event, &live, &links, &asks, &from_browser, &store, changed);
                schedule::on_event(&handle, &event);
                background::on_event(&handle, &event);
                complete::on_event(&handle, &event);
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
            let run_now = handle.state::<schedule::SharedAfter>().run_now.clone();
            let waiting = tokio::task::spawn_blocking(move || {
                store.load().map_err(|e| e.to_string()).map(|q| {
                    // No runner looks meanwhile: a queue started now and
                    // paused at once leaves the set here (review of #125).
                    schedule::prune_run_now(&run_now, &q);
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

/// What the window does with an event of its queue runner: records it for
/// the page, and asks the browser of a download that waits for it.
pub(crate) fn on_queue_event(
    event: &QueueEvent,
    live: &Arc<Live>,
    links: &zajil_host::Links,
    asks: &Arc<BrowserAsks>,
    from_browser: &zajil_core::queue::FromBrowserById,
    store: &QueueStore,
    changed: impl Fn() + Send + 'static,
) {
    live.record(event);
    match event {
        QueueEvent::NeedsBrowser { item, url, source } => {
            ask_browser(links, asks, live, from_browser, *item, url, source, changed);
        }
        // A download done: its browser forgets its ticket at once.
        QueueEvent::Finished { .. } => send_keep(links, store, None),
        _ => {}
    }
}

/// The window's queue runner: categories, the settings' connections, and
/// the browser's headers of each download the extension sent.
pub(crate) fn queue_runner(
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
