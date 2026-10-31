//! The main window, freed while Zajil runs in the tray (owner's request for
//! 1.2.3): its web page alone took most of Zajil's memory (about 170 MB of
//! WebView2 on Windows, against 20 MB for the program), for a window nobody
//! sees. Closed into the tray (or the Dock), the window is destroyed, with
//! the hidden windows made beforehand; the downloads go on without a page.
//!
//! Whatever showed the window makes it again from the configuration,
//! with the same label (`main`): the tray, the Dock, a second start, the
//! extension, a menu command. Its page reads everything again when it
//! loads (the list, the settings, an update or extension notice, the video
//! window): nothing it shows lives only in the page. What was sent to the
//! page while there was none and must not be lost (the tray's «Quit», which
//! asks first while downloads run; «New category») waits here, and goes
//! once the new page has drawn its list and listens ([`painted`]).
//!
//! A window that the main one owns on Windows (`windows::open_beside_main`)
//! would go with it: while one is open the main window is only hidden, and
//! freed once the last of them closes.

use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{Emitter, Manager};

use crate::App;

/// The main window's label, in the configuration.
pub const LABEL: &str = "main";

/// The main window's state while Zajil runs.
#[derive(Default)]
pub struct MainWindow {
    /// Closed into the tray: hidden, to be freed.
    hidden: AtomicBool,
    /// Freed: there is no main window, on purpose.
    freed: AtomicBool,
    /// Being made again.
    making: AtomicBool,
    /// What waits for the next main page: an event and its payload.
    waiting: Mutex<Vec<(String, serde_json::Value)>>,
    /// The windows the main one owns (see the module's text), by label.
    owned: Mutex<HashSet<String>>,
}

impl MainWindow {
    /// Whether the main window was freed on purpose: Zajil goes on with no
    /// window at all, and must not quit for that.
    pub fn freed(&self) -> bool {
        self.freed.load(Ordering::SeqCst)
    }
}

/// The main window was closed into the tray (`closing.rs`, which hid it):
/// it is freed, unless a window it owns is open.
pub fn closed<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    handle
        .state::<MainWindow>()
        .hidden
        .store(true, Ordering::SeqCst);
    // Not from inside the window's own close event.
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move { free(&handle) });
}

/// Frees the main window if it was closed into the tray, is still hidden,
/// and owns no window; and the hidden windows made beforehand with it
/// (they are made again with the next main page).
fn free<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    let state = handle.state::<MainWindow>();
    let app = handle.state::<App>();
    if !state.hidden.load(Ordering::SeqCst)
        || state.freed()
        || !state.owned.lock().unwrap().is_empty()
        || app.exiting.load(Ordering::SeqCst)
        || crate::in_scene(handle)
    {
        return;
    }
    let Some(main) = handle.get_webview_window(LABEL) else {
        return;
    };
    // Before the window goes: its going must not close the others, nor end
    // Zajil (`windows::close_with_main`, `lib.rs`).
    state.freed.store(true, Ordering::SeqCst);
    // The next page is a first one again: it makes the spare windows, and
    // the CI's checks read its «ready».
    app.page_ready.store(false, Ordering::SeqCst);
    crate::windows::drop_spares(handle);
    // What the page said, Zajil says meanwhile (`in_tray.rs`).
    crate::in_tray::start(handle);
    match main.destroy() {
        Ok(()) => eprintln!("zajil: the main window is freed while Zajil runs in the tray"),
        Err(err) => {
            state.freed.store(false, Ordering::SeqCst);
            eprintln!("zajil: the main window could not be freed: {err}");
        }
    }
}

/// Window `label`, owned by the main one, opened (`windows::open_beside_main`):
/// the main window stays until it closes.
pub fn owns<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, window: &tauri::WebviewWindow<R>) {
    let label = window.label().to_owned();
    handle
        .state::<MainWindow>()
        .owned
        .lock()
        .unwrap()
        .insert(label.clone());
    let handle = handle.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            handle
                .state::<MainWindow>()
                .owned
                .lock()
                .unwrap()
                .remove(&label);
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move { free(&handle) });
        }
    });
}

/// Shows the main window and brings it to the front, making it again if
/// it was freed.
pub fn show<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    let state = handle.state::<MainWindow>();
    if !state.freed()
        && let Some(window) = handle.get_webview_window(LABEL)
    {
        state.hidden.store(false, Ordering::SeqCst);
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    if state.making.swap(true, Ordering::SeqCst) {
        return;
    }
    // Never on the main thread (the tray's and the menus' events run
    // there): on Windows, WebView2 waits for it (`windows::open`).
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(err) = make(&handle).await {
            eprintln!("zajil: the main window could not be made again: {err}");
        }
        handle
            .state::<MainWindow>()
            .making
            .store(false, Ordering::SeqCst);
    });
}

/// Makes the main window again, from the configuration, as at the start.
async fn make<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) -> tauri::Result<()> {
    // The one being freed goes first: the label is taken until then.
    for _ in 0..40 {
        if handle.get_webview_window(LABEL).is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let config = handle
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == LABEL)
        .cloned()
        .ok_or_else(|| tauri::Error::WindowNotFound)?;
    let window = tauri::WebviewWindowBuilder::from_config(handle, &config)?.build()?;
    let state = handle.state::<MainWindow>();
    state.hidden.store(false, Ordering::SeqCst);
    state.freed.store(false, Ordering::SeqCst);
    // What the first main window had from the setup.
    crate::closing::watch(handle);
    crate::windows::close_with_main(handle);
    crate::shutdown::hook_main(handle);
    let _ = window.set_focus();
    eprintln!("zajil: the main window is made again");
    Ok(())
}

/// Sends `event` to the main page: at once if there is one, or to the next
/// one, made now, once it listens ([`painted`]). The window shows.
pub fn send<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    event: &str,
    payload: impl serde::Serialize,
) {
    let Ok(payload) = serde_json::to_value(payload) else {
        return;
    };
    let state = handle.state::<MainWindow>();
    if !state.freed() && handle.get_webview_window(LABEL).is_some() {
        show(handle);
        let _ = handle.emit_to(LABEL, event, payload);
        return;
    }
    state
        .waiting
        .lock()
        .unwrap()
        .push((event.to_owned(), payload));
    show(handle);
}

/// The main page drew its list and listens: what waited for it goes.
pub fn painted<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    let waiting = std::mem::take(&mut *handle.state::<MainWindow>().waiting.lock().unwrap());
    for (event, payload) in waiting {
        let _ = handle.emit_to(LABEL, &event, payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tauri's mock runtime keeps a destroyed window in its registry and
    // sends no window events, so these test the decisions; the window
    // freed and made again in the real app is `tests/close_resume.rs`'s
    // `the_main_window_is_freed_in_the_tray_and_comes_back`.

    /// Closed into the tray, the main window is freed, unless a window it
    /// owns is open; and not while Zajil quits.
    #[test]
    fn the_main_window_is_freed_unless_a_window_it_owns_is_open() {
        let data = tempfile::tempdir().unwrap();
        let app = crate::mock_app(data.path());
        let handle = app.handle().clone();
        let state = handle.state::<MainWindow>();

        // Not closed into the tray: kept.
        free(&handle);
        assert!(!state.freed());

        state.hidden.store(true, Ordering::SeqCst);
        state.owned.lock().unwrap().insert("complete-1".into());
        free(&handle);
        assert!(!state.freed(), "freed while a window it owns was open");

        state.owned.lock().unwrap().clear();
        handle.state::<App>().exiting.store(true, Ordering::SeqCst);
        free(&handle);
        assert!(!state.freed(), "freed while Zajil quits");

        handle.state::<App>().exiting.store(false, Ordering::SeqCst);
        handle
            .state::<App>()
            .page_ready
            .store(true, Ordering::SeqCst);
        free(&handle);
        assert!(state.freed());
        // The next page is a first one again.
        assert!(!handle.state::<App>().page_ready.load(Ordering::SeqCst));
    }

    /// What is sent to the main page while it is freed waits, and goes
    /// once the next page listens, once.
    #[test]
    fn what_waits_for_the_main_page_goes_once_it_listens() {
        let data = tempfile::tempdir().unwrap();
        let app = crate::mock_app(data.path());
        let handle = app.handle().clone();
        let state = handle.state::<MainWindow>();
        // Freed: as `free` leaves it. The window it would make again is
        // the mock's concern, not this test's.
        state.freed.store(true, Ordering::SeqCst);
        state.making.store(true, Ordering::SeqCst);
        send(&handle, "menu-command", "quit");
        send(&handle, "main-view", "categories");
        assert_eq!(
            *state.waiting.lock().unwrap(),
            [
                ("menu-command".to_owned(), serde_json::json!("quit")),
                ("main-view".to_owned(), serde_json::json!("categories")),
            ]
        );
        painted(&handle);
        assert!(state.waiting.lock().unwrap().is_empty());
    }

    /// The new main page reads its state from Rust as the first one did:
    /// an update found while there was no page is still there.
    #[test]
    fn the_new_main_page_reads_its_state_from_rust() {
        let data = tempfile::tempdir().unwrap();
        let app = crate::mock_app(data.path());
        let handle = app.handle().clone();
        handle
            .state::<MainWindow>()
            .freed
            .store(true, Ordering::SeqCst);
        crate::update::found_for_test(&handle, "9.9.9");
        let window = handle.get_webview_window(LABEL).unwrap();
        let notice = crate::invoke_for_test(&window, "update_notice", serde_json::json!({}));
        assert_eq!(notice.unwrap()["version"], "9.9.9");
        let snapshot = crate::invoke_for_test(&window, "snapshot", serde_json::json!({}));
        assert!(snapshot.unwrap()["downloads"].is_array());
    }
}
