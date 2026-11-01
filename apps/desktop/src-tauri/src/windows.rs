//! The add windows (M5.5, the design of #83, windows 2 and 3): «Add a
//! link» and «Download file info» are windows of their own, as in IDM, not
//! dialogs inside the main window. The page draws them too
//! (`index.html?win=…`); what each one starts with waits here, so no link
//! goes into a window's address. They have their own capability (`add`):
//! listening to Zajil's events and the system's folder picker, nothing
//! else.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use ts_rs::TS;

use crate::App;

/// The add windows' capability: listening to events and the folder
/// picker. Added when the app starts, for those two windows only.
pub const CAPABILITY: &str = include_str!("../runtime-capabilities/add.json");

/// The progress windows' capability: listening to events, and their own
/// title (the percentage and the name) and size (with or without the
/// details); the system's question before a program opens (#84).
pub const PROGRESS_CAPABILITY: &str = include_str!("../runtime-capabilities/progress.json");

pub const LINK: &str = "add-link";
pub const INFO: &str = "add-info";

/// The «Add a link» window's size, in logical pixels, and its least: the
/// page makes it taller when what it shows needs more ([`fit_window`]).
/// Its link, its note and its buttons, with room under them.
pub const LINK_SIZE: (f64, f64) = (560.0, 190.0);
/// The «Download file info» window's, the same way: every row (the line
/// about the file, the link, the category, «Save as», the queue, the
/// description and the buttons), drawn from its first frame, so it opens
/// at the size it keeps and never grows when the server answers (the
/// owner's notes: it opened small, then grew).
pub const INFO_SIZE: (f64, f64) = (560.0, 316.0);

/// The settings window (the owner's request: a window of its own with a
/// list of sections at its side, as in other download managers).
pub const SETTINGS: &str = "settings";
pub const SETTINGS_SIZE: (f64, f64) = (860.0, 620.0);

/// The settings window's capability: its dialogs and launching at login.
pub const SETTINGS_CAPABILITY: &str = include_str!("../runtime-capabilities/settings.json");

/// A progress window's size with its details shown (design window 4).
pub const PROGRESS_SIZE: (f64, f64) = (600.0, 640.0);

/// The progress window of download `id`.
pub fn progress_label(id: u64) -> String {
    format!("progress-{id}")
}

/// What «Add a link» starts with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PendingLink {
    pub url: String,
    /// It came from the clipboard: the window says so.
    pub from_clipboard: bool,
}

/// What a «Download file info» window starts with: the link, and the
/// browser's download it is for (`offers.rs`), if it came from there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InfoStart {
    pub url: String,
    #[ts(type = "number | null")]
    pub offer: Option<u64>,
}

/// What each add window starts with: «Download file info» windows by
/// label (one per download from the browser, as in IDM).
#[derive(Debug, Default)]
pub struct Pending {
    link: Option<PendingLink>,
    info: std::collections::HashMap<String, InfoStart>,
}

impl Pending {
    pub fn set_info(&mut self, label: &str, start: InfoStart) {
        self.info.insert(label.to_owned(), start);
    }

    pub fn forget_info(&mut self, label: &str) {
        self.info.remove(label);
    }
}

pub type Shared = Mutex<Pending>;

/// Opens window `label` (`win` in the page's address), or brings it to the
/// front if it is open.
///
/// Only from an async command or another thread, never from a command
/// that is not async (those run on the main thread) nor an event handler:
/// on Windows, WebView2 then waits for the main thread, which waits for
/// it, and the window's page never loads (Tauri's documented deadlock;
/// the «Add link» window of the end-to-end test of #226 stayed blank).
pub(crate) fn open<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    label: &str,
    title: &str,
    size: (f64, f64),
) -> tauri::Result<()> {
    open_page(handle, label, label, title, size, false)
}

/// [`open`], with `page` after `win=` in the page's address (a window of
/// a download: its kind and id, never a link). `owned`: the window belongs
/// to the main one (see [`open_beside_main`]).
pub(crate) fn open_page<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    label: &str,
    page: &str,
    title: &str,
    (width, height): (f64, f64),
    owned: bool,
) -> tauri::Result<()> {
    crate::measure::asked(label);
    if let Some(window) = find(handle, label) {
        let _ = window.unminimize();
        window.show()?;
        return window.set_focus();
    }
    let least = (
        width,
        height.min(if label.starts_with(INFO) || label == LINK {
            height
        } else {
            320.0
        }),
    );
    // A window made beforehand, hidden, with its page loaded: it only
    // takes its role and shows (the owner's request: windows open at once).
    if let Some(window) = take_spare(handle, label, page) {
        // What the page misses of the window, once learned: the window
        // opens at the height its page fits, not 32 pixels short of it.
        let missing = missing(Kind::of(label));
        window.set_title(title)?;
        window.set_min_size(Some(tauri::LogicalSize::new(least.0, least.1 + missing)))?;
        window.set_size(tauri::LogicalSize::new(width, height + missing))?;
        window.center()?;
        on_gone(handle, &window, label);
        let _ = handle.emit_to(
            window.label(),
            "window-role",
            Role {
                label: label.to_owned(),
                page: page.to_owned(),
            },
        );
        // A progress window shows at once: it can be minimized next
        // (`addprefs::show_started`), and its size never changes.
        if !matches!(Kind::of(label), Some(Kind::Link | Kind::Info)) {
            window.show()?;
            return window.set_focus();
        }
        // An add window shows once its page has drawn the role, at its
        // height ([`window_ready`]), so its first frame is the whole
        // window, not an empty or short one that fills in and grows (the
        // owner's notes). Shown anyway if the page does not say so soon: a
        // page asleep must not keep the window from the user.
        show_at_latest(&window);
        return Ok(());
    }
    let builder = WebviewWindowBuilder::new(
        handle,
        label,
        WebviewUrl::App(format!("index.html?win={page}").into()),
    );
    // On Windows a window Zajil opens by itself is owned by the main one:
    // a close sent to the program (`taskkill`, an installer) goes to its
    // main window, the first unowned one, which would otherwise be this
    // newer window, and Zajil would go on (review of #137: the close tests).
    // Owned windows also go with their owner. Windows only: elsewhere a
    // parent ties the window to the main one, hidden in the tray with it.
    #[cfg(windows)]
    let builder = match handle.get_webview_window("main") {
        Some(main) if owned => builder.owner(&main)?,
        _ => builder,
    };
    #[cfg(not(windows))]
    let _ = owned;
    // An add window made now is hidden until its page is drawn, as a
    // spare one ([`window_ready`]).
    let add = matches!(Kind::of(label), Some(Kind::Link | Kind::Info));
    let missing = missing(Kind::of(label));
    let window = builder
        .title(title)
        .inner_size(width, height + missing)
        .min_inner_size(least.0, least.1 + missing)
        .visible(!add)
        .resizable(true)
        .maximizable(false)
        .minimizable(false)
        .center()
        .focused(true)
        .build()?;
    on_gone(handle, &window, label);
    if add {
        show_at_latest(&window);
    }
    Ok(())
}

/// What a window in role `label` leaves when it closes. The info window:
/// a check of the server still running is given up (as the dialog's
/// cancel did), and a download it started and did not place is removed
/// (`background.rs`).
fn on_gone<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    window: &tauri::WebviewWindow<R>,
    label: &str,
) {
    if !label.starts_with(INFO) {
        return;
    }
    let (app, label) = (handle.clone(), label.to_owned());
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            if label == INFO
                && let Some(stop) = app.state::<App>().probing.lock().unwrap().take()
            {
                stop.request();
            }
            crate::background::window_gone(&app, &label);
        }
    });
}

/// The role a window made beforehand takes: the label it stands for, and
/// its page (`win=` in the address of a window made for it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Role {
    pub label: String,
    pub page: String,
}

/// The kinds of window that can be made beforehand: the ones that open
/// most, and that the user waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Link,
    Info,
    Progress,
}

impl Kind {
    /// The ones made beforehand: «Download file info» alone, the window
    /// opened most (every download from the browser, and from «Add a
    /// link»). Each spare is a page of its own, about 20 MB on macOS; the
    /// others open in about 0.1 s without one (the owner's choice, after
    /// measuring three: 56 MB).
    const SPARE: [Kind; 1] = [Kind::Info];

    /// The kind of the window in role `label`.
    fn of(label: &str) -> Option<Kind> {
        if label == LINK {
            Some(Kind::Link)
        } else if label.starts_with(INFO) {
            Some(Kind::Info)
        } else if label.starts_with("progress-") {
            Some(Kind::Progress)
        } else {
            None
        }
    }

    /// Its spare windows' labels: within its capability's (`add-link-*`,
    /// `add-info-*`, `progress-*`), and never a role's.
    fn prefix(self) -> &'static str {
        match self {
            Kind::Link => "add-link-spare-",
            Kind::Info => "add-info-spare-",
            Kind::Progress => "progress-spare-",
        }
    }

    fn size(self) -> (f64, f64) {
        match self {
            Kind::Link => LINK_SIZE,
            Kind::Info => INFO_SIZE,
            Kind::Progress => PROGRESS_SIZE,
        }
    }
}

/// The windows made beforehand: one spare of each kind, and the role of
/// each one taken.
#[derive(Default)]
pub struct Pool {
    spares: Mutex<std::collections::HashMap<Kind, String>>,
    /// The role of each window taken, by its own label.
    roles: Mutex<std::collections::HashMap<String, Role>>,
    next: std::sync::atomic::AtomicU64,
}

/// The role of `window`: the label it stands for, or its own.
pub fn role_of<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) -> String {
    window
        .state::<Pool>()
        .roles
        .lock()
        .unwrap()
        .get(window.label())
        .map_or_else(|| window.label().to_owned(), |r| r.label.clone())
}

/// The window in role `label`: one made beforehand that took it, or one
/// made for it.
pub fn find<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    label: &str,
) -> Option<tauri::WebviewWindow<R>> {
    let own = handle
        .state::<Pool>()
        .roles
        .lock()
        .unwrap()
        .iter()
        .find(|(_, role)| role.label == label)
        .map(|(own, _)| own.clone());
    handle.get_webview_window(own.as_deref().unwrap_or(label))
}

/// The spare of `label`'s kind, now in role `label` (another is made).
fn take_spare<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    label: &str,
    page: &str,
) -> Option<tauri::WebviewWindow<R>> {
    let kind = Kind::of(label)?;
    let pool = handle.state::<Pool>();
    let own = pool.spares.lock().unwrap().remove(&kind)?;
    let window = handle.get_webview_window(&own)?;
    pool.roles.lock().unwrap().insert(
        own,
        Role {
            label: label.to_owned(),
            page: page.to_owned(),
        },
    );
    refill(handle, kind, std::time::Duration::from_millis(400));
    Some(window)
}

/// How long a spare window that took a role waits for its page to say it
/// is drawn before it shows anyway.
const SHOW_AT_LATEST: std::time::Duration = std::time::Duration::from_millis(400);

/// Shows add window `window` after [`SHOW_AT_LATEST`] if its page has not
/// said it is drawn by then ([`window_ready`]).
fn show_at_latest<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    let late = window.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SHOW_AT_LATEST).await;
        show_once(&late);
    });
}

/// Shows `window` and brings it to the front, unless it shows already.
fn show_once<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    if window.is_visible().unwrap_or(true) {
        return;
    }
    crate::measure::shown(window);
    let _ = window.show();
    let _ = window.set_focus();
}

/// The page of an add window has drawn its rows, at its height: the
/// window shows ([`open_page`]).
#[tauri::command]
pub fn window_ready<R: tauri::Runtime>(window: tauri::WebviewWindow<R>) {
    if matches!(Kind::of(&role_of(&window)), Some(Kind::Link | Kind::Info)) {
        show_once(&window);
    }
}

/// The role of the window that asks, once it has one (`None`: a spare).
#[tauri::command]
pub fn window_role<R: tauri::Runtime>(window: tauri::WebviewWindow<R>) -> Option<Role> {
    window
        .state::<Pool>()
        .roles
        .lock()
        .unwrap()
        .get(window.label())
        .cloned()
}

/// Makes the spare windows, hidden, a little after the main window's page
/// is up (it comes first). Not in a UI test scene, whose windows are drawn
/// in the main one.
pub fn warm<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    if crate::in_scene(handle) || !crate::measure::pool_on() {
        return;
    }
    for kind in Kind::SPARE {
        refill(handle, kind, std::time::Duration::from_millis(300));
    }
}

/// A new spare of `kind` after `delay`, if it has none, while the main
/// window is there and Zajil is not quitting. From another thread: never
/// on the main one ([`open`]).
fn refill<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, kind: Kind, delay: std::time::Duration) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        let up = handle.get_webview_window("main").is_some()
            && !handle
                .state::<App>()
                .exiting
                .load(std::sync::atomic::Ordering::SeqCst);
        let pool = handle.state::<Pool>();
        if !up || pool.spares.lock().unwrap().contains_key(&kind) {
            return;
        }
        let n = pool.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let label = format!("{}{n}", kind.prefix());
        let (width, height) = kind.size();
        let made = WebviewWindowBuilder::new(
            &handle,
            &label,
            WebviewUrl::App("index.html?win=spare".into()),
        )
        .title("Zajil")
        .inner_size(width, height)
        .resizable(true)
        .maximizable(false)
        .minimizable(false)
        .center()
        .visible(false)
        .focused(false)
        .build();
        match made {
            Ok(window) => {
                let (app, own) = (handle.clone(), label.clone());
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::Destroyed = event {
                        let pool = app.state::<Pool>();
                        pool.roles.lock().unwrap().remove(&own);
                        pool.spares.lock().unwrap().retain(|_, l| *l != own);
                    }
                });
                pool.spares.lock().unwrap().insert(kind, label);
            }
            Err(err) => eprintln!("zajil: a window made beforehand: {err}"),
        }
    });
}

/// «Add a link»: with `url` (a copied link, or none), titled `title` in
/// the page's language. An open one takes the new link.
#[tauri::command]
pub async fn open_add_link<R: tauri::Runtime>(
    pending: State<'_, Shared>,
    url: Option<String>,
    from_clipboard: bool,
    title: String,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    if crate::in_scene(&handle) {
        return Ok(());
    }
    let link = PendingLink {
        url: url.unwrap_or_default(),
        from_clipboard,
    };
    pending.lock().unwrap().link = Some(link.clone());
    if let Some(window) = find(&handle, LINK) {
        let _ = handle.emit_to(window.label(), "pending-link", link);
    }
    open(&handle, LINK, &title, LINK_SIZE).map_err(|e| e.to_string())
}

/// The link «Add a link» starts with.
#[tauri::command]
pub fn pending_link(pending: State<'_, Shared>) -> Option<PendingLink> {
    pending.lock().unwrap().link.clone()
}

/// «Download file info» for `url`, titled `title`; «Add a link» closes.
/// One info window at a time: while one is open, it comes to the front and
/// [`INFO_OPEN`] is the answer.
#[tauri::command]
pub async fn open_add_info<R: tauri::Runtime>(
    pending: State<'_, Shared>,
    url: String,
    title: String,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    if crate::in_scene(&handle) {
        return Ok(());
    }
    // One at a time: another link would be lost behind the open one
    // (review of #104), so the link window stays and says so.
    if let Some(info) = find(&handle, INFO) {
        let _ = info.set_focus();
        return Err(INFO_OPEN.into());
    }
    if let Some(link) = find(&handle, LINK) {
        let _ = link.close();
    }
    pending
        .lock()
        .unwrap()
        .set_info(INFO, InfoStart { url, offer: None });
    open(&handle, INFO, &title, INFO_SIZE).map_err(|e| e.to_string())
}

/// Makes the add window that asks as tall as what its page shows
/// (`height`, in logical pixels from the top of the page, measured by the
/// page whenever it changes), and never shorter: the owner's notes on 1.0,
/// the content was cut at the top and a scroll bar came at a fixed 260.
/// Never shorter than it opened either, whatever the page says (the 1.0
/// draft: a page scrolled by the focus measured itself short, and the
/// window shrank under its content). Its width stays the user's. In a
/// scene, the main window that draws it fits it the same way, without the
/// least size.
///
/// `page`: the page's own height now. On macOS it can be less than the
/// window's content (the CI's windows: 32 pixels less, though Tauri gave
/// the page's frame as the whole content); what a kind of window misses
/// is learned at its first call, before anything resized it, and added
/// from then on.
#[tauri::command]
pub async fn fit_window<R: tauri::Runtime>(
    window: tauri::WebviewWindow<R>,
    height: f64,
    page: f64,
) -> Result<(), String> {
    if !(height.is_finite() && height > 0.0 && page.is_finite() && page > 0.0) {
        return Err(format!("not a height: {height}, {page}"));
    }
    let label = role_of(&window);
    let label = label.as_str();
    let (kind, least, opened) = if label == LINK {
        (LINK, Some(LINK_SIZE.0), LINK_SIZE.1)
    } else if label.starts_with(INFO) {
        (INFO, Some(INFO_SIZE.0), INFO_SIZE.1)
    } else if let Some((_, opened)) = crate::scene_size(window.app_handle())
        && label == "main"
    {
        ("main", None, opened)
    } else {
        return Err("not an add window".into());
    };
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let now = window
        .inner_size()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(scale);
    let missing = *MISSING
        .lock()
        .unwrap()
        .entry(kind)
        .or_insert((now.height - page).clamp(0.0, 100.0));
    let height = height.max(opened) + missing;
    if (height - now.height).abs() < 1.0 {
        return Ok(());
    }
    crate::measure::resized(&window, now.height, height);
    // The least first: a window going shorter is not held by the old one.
    if let Some(least) = least {
        window
            .set_min_size(Some(tauri::LogicalSize::new(least, height)))
            .map_err(|e| e.to_string())?;
    }
    window
        .set_size(tauri::LogicalSize::new(now.width, height))
        .map_err(|e| e.to_string())
}

/// What the page of a window of `kind` misses of its content's height, if
/// [`fit_window`] learned it already (else 0).
fn missing(kind: Option<Kind>) -> f64 {
    let kind = match kind {
        Some(Kind::Link) => LINK,
        Some(Kind::Info) => INFO,
        _ => return 0.0,
    };
    MISSING.lock().unwrap().get(kind).copied().unwrap_or(0.0)
}

/// What the page of each kind of add window misses of its content's
/// height ([`fit_window`]).
static MISSING: std::sync::LazyLock<Mutex<std::collections::HashMap<&'static str, f64>>> =
    std::sync::LazyLock::new(Default::default);

/// What [`open_add_info`] answers while «Download file info» is open: the
/// page says it in the user's language.
pub const INFO_OPEN: &str = "info-open";

/// Opens window `label` on `page` only while the main window is there and
/// Zajil is not quitting: a window that Zajil opens by itself (a download
/// finished) must not keep a quitting Zajil alive. Closing the main window
/// ends Zajil once no window is left (the close tests on Windows, review
/// of #137): a window that opened just after it would stop that. If the
/// main window goes while this one opens, this one goes too.
pub(crate) fn open_beside_main<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    label: &str,
    page: &str,
    title: &str,
    size: (f64, f64),
) -> tauri::Result<()> {
    // Freed in the tray (`main_window.rs`), it is still there for this.
    let main_is_up = |h: &tauri::AppHandle<R>| {
        (h.get_webview_window("main").is_some()
            || h.state::<crate::main_window::MainWindow>().freed())
            && !h
                .state::<App>()
                .exiting
                .load(std::sync::atomic::Ordering::SeqCst)
    };
    if !main_is_up(handle) {
        return Ok(());
    }
    open_page(handle, label, page, title, size, true)?;
    if let Some(window) = find(handle, label) {
        crate::main_window::owns(handle, &window);
    }
    if !main_is_up(handle)
        && let Some(window) = find(handle, label)
    {
        window.destroy()?;
    }
    Ok(())
}

/// The other windows close when the main one does: without it the program
/// would go on with no main window to come back to (review of #104).
pub fn close_with_main<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    let Some(main) = handle.get_webview_window("main") else {
        return;
    };
    let app = handle.clone();
    main.on_window_event(move |event| {
        // Freed in the tray, it is no end (`main_window.rs`).
        if let tauri::WindowEvent::Destroyed = event
            && !app.state::<crate::main_window::MainWindow>().freed()
        {
            for (label, window) in app.webview_windows() {
                if label != "main" {
                    let _ = window.close();
                }
            }
        }
    });
}

/// What the «Download file info» window that asks is for.
#[tauri::command]
pub fn pending_info<R: tauri::Runtime>(
    pending: State<'_, Shared>,
    window: tauri::WebviewWindow<R>,
) -> Option<InfoStart> {
    pending.lock().unwrap().info.get(&role_of(&window)).cloned()
}

/// Closes the window that asks.
#[tauri::command]
pub fn close_window<R: tauri::Runtime>(window: tauri::WebviewWindow<R>) -> Result<(), String> {
    window.close().map_err(|e| e.to_string())
}

/// The progress window of download `id` (design window 4), one per
/// download as in IDM, titled `title`; an open one comes to the front. The
/// page reads the download's id from the window's label.
#[tauri::command]
pub async fn open_progress<R: tauri::Runtime>(
    id: u64,
    title: String,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    if crate::in_scene(&handle) {
        return Ok(());
    }
    open(&handle, &progress_label(id), &title, PROGRESS_SIZE).map_err(|e| e.to_string())
}

/// The settings window, titled `title`; an open one comes to the front.
#[tauri::command]
pub async fn open_settings<R: tauri::Runtime>(
    title: String,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    if crate::in_scene(&handle) {
        return Ok(());
    }
    open(&handle, SETTINGS, &title, SETTINGS_SIZE).map_err(|e| e.to_string())
}

/// Shows the main window on `view` (the categories editor, from «New
/// category»).
#[tauri::command]
pub fn show_main_view<R: tauri::Runtime>(view: String, handle: tauri::AppHandle<R>) {
    crate::main_window::send(&handle, "main-view", view);
}

/// Destroys the spare windows (`main_window.rs`: freed with the main one;
/// the next main page makes them again).
pub fn drop_spares<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    let spares: Vec<String> = handle
        .state::<Pool>()
        .spares
        .lock()
        .unwrap()
        .drain()
        .map(|(_, label)| label)
        .collect();
    for label in spares {
        if let Some(window) = handle.get_webview_window(&label) {
            let _ = window.destroy();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One window made beforehand, «Download file info»; the others are
    /// made when they open (the owner's choice).
    #[test]
    fn only_download_file_info_is_made_beforehand() {
        assert_eq!(Kind::SPARE, [Kind::Info]);
        assert_eq!(Kind::of(INFO), Some(Kind::Info));
        assert_eq!(Kind::of(&format!("{INFO}-3")), Some(Kind::Info));
    }

    #[test]
    fn a_window_starts_with_nothing_waiting() {
        let pending = Pending::default();
        assert!(pending.link.is_none() && pending.info.is_empty());
        assert_eq!(LINK, "add-link");
        assert_eq!(INFO, "add-info");
        assert_eq!(progress_label(7), "progress-7");
    }

    /// The progress windows may listen, set their own title and size, and
    /// ask before a program opens; nothing more.
    #[test]
    fn the_progress_windows_have_only_their_own_permissions() {
        let capability: serde_json::Value = serde_json::from_str(PROGRESS_CAPABILITY).unwrap();
        assert_eq!(capability["windows"], serde_json::json!(["progress-*"]));
        assert_eq!(
            capability["permissions"],
            serde_json::json!([
                "core:event:default",
                "core:window:allow-set-title",
                "core:window:allow-set-size",
                "dialog:allow-message"
            ])
        );
    }

    /// The add windows may listen and pick a folder, and nothing more; the
    /// main window's permissions are not theirs.
    #[test]
    fn the_add_windows_have_only_their_own_permissions() {
        let capability: serde_json::Value = serde_json::from_str(CAPABILITY).unwrap();
        assert_eq!(
            capability["windows"],
            serde_json::json!([LINK, "add-link-*", INFO, "add-info-*"])
        );
        assert_eq!(
            capability["permissions"],
            serde_json::json!(["core:event:default", "dialog:allow-open"])
        );
    }

    /// The main window is the same on every system but for its title bar
    /// (the design of #83): hidden under the page on macOS, none on Windows
    /// and Linux, where the page draws it.
    #[test]
    fn the_main_window_differs_by_system_only_in_its_title_bar() {
        let read = |name: &str| -> serde_json::Value {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        };
        let base = read("tauri.conf.json")["app"]["windows"][0].clone();
        for (name, own) in [
            (
                "tauri.macos.conf.json",
                serde_json::json!({"titleBarStyle": "Overlay", "hiddenTitle": true}),
            ),
            (
                "tauri.windows.conf.json",
                serde_json::json!({"decorations": false}),
            ),
            (
                "tauri.linux.conf.json",
                serde_json::json!({"decorations": false}),
            ),
        ] {
            let mut expected = base.clone();
            for (k, v) in own.as_object().unwrap() {
                expected[k] = v.clone();
            }
            assert_eq!(
                read(name)["app"]["windows"],
                serde_json::json!([expected]),
                "{name}"
            );
        }
    }
}
