//! Zajil's desktop interface (M4, ADR-0003, ADR-0008): a Tauri 2 window
//! over the engine. Like the command line, it holds no download logic
//! (ARCHITECTURE §1): it runs the queues with `zajil_core::queue::Runner`,
//! sends the user's commands to the queues file, and shows what happens.
//!
//! The page asks for a [`view::Snapshot`] when told that something changed
//! (event `downloads-changed`), and the running rows at each progress
//! tick (`downloads-progress`, at most twice a second).

#![forbid(unsafe_code)]

mod addprefs;
mod asks;
mod background;
mod batch;
mod clipboard;
mod closing;
mod complete;
mod downloads;
mod dropbox;
mod extension;
mod files;
mod grab;
mod in_tray;
mod instance;
mod integration;
mod live;
mod main_window;
mod measure;
mod menu;
mod offers;
mod problem;
mod refresh;
mod removal;
mod runner;
mod saving;
mod schedule;
mod settings;
mod shutdown;
mod sounds;
mod tray;
#[cfg(feature = "ui-test")]
mod ui_test;
mod update;
mod video;
pub mod view;
mod windows;

/// The add windows' sizes when they open: the end-to-end test checks the
/// windows against them.
pub use windows::{INFO_SIZE, LINK_SIZE};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use tauri::{Emitter, Manager};
use zajil_core::category::CategoryStore;
use zajil_core::queue::QueueStore;
use zajil_core::settings::SettingsStore;
use zajil_core::{Engine, Options, Stop};
use zajil_platform::{AppPaths, SystemPaths};

use crate::live::Live;

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
    /// What «Copy link» put on the clipboard: the monitoring does not offer
    /// it back (M5.5).
    copied: Arc<std::sync::Mutex<String>>,
    /// The clipboard «Copy link» writes with, kept open: on Linux the text
    /// lives only while its writer does, with no clipboard manager.
    copier: Arc<std::sync::Mutex<Option<zajil_platform::clipboard::SystemClipboard>>>,
    /// The data folder (`--data-dir` or the system's).
    data_dir: PathBuf,
    /// The browser's cookies and page of each download the extension sent,
    /// by item: in memory only (owner's decision on #7).
    from_browser: zajil_core::queue::FromBrowserById,
    /// The browsers' extensions linked to this window, which it asks for
    /// cookies again after a restart (M5).
    links: zajil_host::Links,
    /// The downloads whose browser is being asked, or was asked lately.
    asks: Arc<asks::BrowserAsks>,
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
    refreshing: refresh::Refreshing,
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

/// The first line Zajil says: its release's version (`zajil_core::VERSION`;
/// this crate's own is always 0.0.0, #248) and its process.
fn start_line(process: u32) -> String {
    format!(
        "zajil: starting (version {}, process {process})",
        zajil_core::VERSION
    )
}

/// What the commands share, for data folder `data`, with the settings as
/// they are now.
fn new_app(data: &Path, system_download_dir: PathBuf) -> App {
    let settings = SettingsStore::new(data);
    // A damaged settings file is shown when the page asks for it; the
    // downloads run with the defaults meanwhile.
    let loaded = settings.load();
    let connections = loaded
        .as_ref()
        .map(|s| s.connections as usize)
        .unwrap_or(zajil_core::DEFAULT_CONNECTIONS);
    // Off with a damaged settings file, as by default.
    let clipboard = loaded.as_ref().is_ok_and(|s| s.clipboard);
    // The per-site connection limits of the settings (#165): none with a
    // damaged file, as by default.
    let sites = loaded
        .as_ref()
        .map(zajil_core::sites::SiteLimits::from_settings)
        .unwrap_or_default();
    // The user's proxy, read at the start (a change applies at the next).
    let proxy = loaded.as_ref().ok().and_then(|s| s.proxy.clone());
    App {
        engine: Engine::new(Options {
            proxy: proxy.clone(),
            ..Options::default()
        })
        // A proxy the engine cannot use: straight to the servers, as
        // before it was set (the settings say it).
        .or_else(|_| Engine::new(Options::default()))
        .expect("the engine starts")
        .with_site_limits(sites),
        store: QueueStore::new(data),
        categories: CategoryStore::new(data),
        settings,
        system_download_dir,
        connections: Arc::new(AtomicUsize::new(connections)),
        clipboard: Arc::new(AtomicBool::new(clipboard)),
        copied: Arc::default(),
        copier: Arc::default(),
        data_dir: data.to_path_buf(),
        from_browser: Arc::default(),
        links: zajil_host::Links::default(),
        asks: Arc::default(),
        integration: Arc::default(),
        live: Arc::new(Live::default()),
        stop: Stop::new(),
        stopped: tokio::sync::watch::Sender::new(false),
        exiting: AtomicBool::new(false),
        page_ready: AtomicBool::new(false),
        probing: std::sync::Mutex::new(None),
        refreshing: refresh::Refreshing::default(),
    }
}

/// `builder` with the state of Zajil's parts: `app`, and each one's own.
fn managed<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
    app: App,
    data: &Path,
    close_quits: bool,
) -> tauri::Builder<R> {
    builder
        .manage(app)
        .manage(windows::Shared::default())
        .manage(windows::Pool::default())
        .manage(background::Held::default())
        .manage(offers::Offers::default())
        .manage(video::VideoState::default())
        .manage(batch::Pending::default())
        .manage(grab::GrabState::default())
        .manage(closing::Closing::new(data, close_quits))
        .manage(schedule::SharedAfter::default())
        .manage(complete::Show::default())
        .manage(update::Notice::default())
        .manage(update::ExtensionOld::default())
        .manage(main_window::MainWindow::default())
        .manage(in_tray::InTray::default())
}

pub fn run() {
    measure::start();
    // Started from the Finder, the Start menu or a desktop file, what Zajil
    // says goes to its log (~/Library/Logs/zajil and the like).
    if let Ok(paths) = SystemPaths::new()
        && let Err(e) = zajil_platform::console::log_stderr_to(&paths.log_dir().join("zajil.log"))
    {
        eprintln!("zajil: no log file: {e}");
    }
    zajil_platform::process::raise_open_files_limit();
    // Said at once, as each stage of the start below (#220: a start whose
    // window never came said nothing at all).
    eprintln!("{}", start_line(std::process::id()));
    let args: Vec<String> = std::env::args().collect();
    let data = data_dir(&args).unwrap_or_else(|e| panic!("no data folder: {e}"));
    let system_download_dir = SystemPaths::new()
        .map(|p| p.download_dir().to_path_buf())
        .unwrap_or_else(|_| data.join("Downloads"));
    let app = new_app(&data, system_download_dir);
    #[cfg(unix)]
    let signals = shutdown::stop_signals();
    // One Zajil per user: with its window hidden in the tray, opening Zajil
    // again shows it instead of starting a second program on the same
    // queues (review of #109). Not with `--data-dir` (the tests and the
    // screenshots, each on a folder of its own).
    let one_zajil = !args.iter().any(|a| a == "--data-dir");
    // On macOS through Zajil's own channel in its data folder: the plugin
    // there listens at a fixed path in `/tmp`, where another user of the
    // Mac could answer first and keep Zajil from ever opening (#60).
    #[cfg(target_os = "macos")]
    if one_zajil && instance::show_running(&data) {
        return;
    }
    let builder = tauri::Builder::default();
    #[cfg(not(target_os = "macos"))]
    let builder = if one_zajil {
        builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_window(app);
        }))
    } else {
        builder
    };
    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        // Opens a finished download or its folder from Rust only
        // (`open_download`): the page has no opener permission.
        .plugin(tauri_plugin_opener::init())
        // Zajil's own signed update (ADR-0011, `update.rs`): from Rust only,
        // no permission for the page.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));
    let builder = managed(
        builder,
        app,
        &data,
        args.iter().any(|a| a == "--close-quits"),
    );
    #[cfg(feature = "ui-test")]
    let builder = builder.manage(ui_test::UiTest::from_args(&args));
    builder
        // The macOS menu bar's commands go to the page (`menu.rs`).
        .on_menu_event(|handle, event| {
            if let Some(command) = event.id().as_ref().strip_prefix(menu::PREFIX) {
                let front = handle
                    .webview_windows()
                    .into_iter()
                    .find(|(_, w)| w.is_focused().unwrap_or(false))
                    .map_or_else(|| "main".to_owned(), |(label, _)| label);
                let to = menu::menu_target(command, &front);
                // A command of the main window, chosen while another is in
                // front: the main window comes forward to do it (#98: the
                // others ignored everything but Select all).
                // Through `main_window`: a main window freed in the tray
                // is made again, and the command waits for its page.
                if to == "main" {
                    main_window::send(handle, "menu-command", command.to_owned());
                } else {
                    let _ = handle.emit_to(to, "menu-command", command.to_owned());
                }
            }
        })
        .invoke_handler(handlers())
        .setup(move |tauri_app| {
            let handle = tauri_app.handle().clone();
            if let Some(window) = tauri_app.get_webview_window("main") {
                eprintln!(
                    "zajil: the window is made (shown: {}); its page is loading",
                    window.is_visible().unwrap_or(false)
                );
            }
            #[cfg(unix)]
            if let Some(signals) = signals {
                tauri::async_runtime::spawn(shutdown::exit_on_signal(handle.clone(), signals));
            }
            // Not fatal: without it, a Windows shutdown still reaches the
            // stop path through tao's own handling (`RunEvent::Exit`).
            #[cfg(windows)]
            if let Err(err) = shutdown::on_windows_session_end(tauri_app) {
                eprintln!("zajil: cannot watch for the end of the Windows session: {err}");
            }
            tray::build(tauri_app)?;
            extension::serve_extension(tauri_app.handle().clone(), data.clone());
            clipboard::watch_clipboard(tauri_app.handle().clone());
            // The browser integration, at each start: idempotent, and a
            // browser installed since then finds Zajil. Not in a ui-test
            // scene, whose files are demo ones.
            if !in_scene(tauri_app.handle()) {
                let data = data.clone();
                let kept = tauri_app.state::<App>().integration.clone();
                std::thread::spawn(move || {
                    let integration = integration::integrate_browsers(&data);
                    for problem in integration::integration_problems(&integration) {
                        eprintln!("zajil: the browser extension: {problem}");
                    }
                    *kept.lock().unwrap_or_else(|e| e.into_inner()) = Some(integration);
                });
            }
            // The add windows' own permissions (`windows.rs`), and they
            // close with the main window (review of #104).
            tauri_app.add_capability(windows::CAPABILITY)?;
            tauri_app.add_capability(video::CAPABILITY)?;
            tauri_app.add_capability(batch::CAPABILITY)?;
            tauri_app.add_capability(grab::CAPABILITY)?;
            // What a video run or a tools install left when Zajil stopped
            // (#149).
            {
                let app = tauri_app.state::<App>();
                video::clean_leftovers(&app);
                video::load_rows(&app, &tauri_app.state::<video::VideoState>());
                if let Ok(settings) = app.settings.load() {
                    video::keep_at_once(
                        &tauri_app.state::<video::VideoState>(),
                        settings.videos_at_once(),
                    );
                }
            }
            // A video tool the user had in an earlier build (yt-dlp's
            // one-file build) is brought up to date in the background.
            video::update_tools(tauri_app.handle().clone());
            // Videos in queues follow their queues' windows, and the last
            // of a queue brings what the user chose for after it.
            video::watch_queues(tauri_app.handle().clone());
            {
                use tauri::Listener;
                let handle = tauri_app.handle().clone();
                tauri_app.listen_any(video::QUEUE_VIDEO_DONE, move |event| {
                    if let Ok(queue) = serde_json::from_str::<String>(event.payload()) {
                        schedule::video_done(&handle, queue);
                    }
                });
            }
            tauri_app.add_capability(dropbox::CAPABILITY)?;
            // The drop box, if the user left it shown.
            if tauri_app.state::<closing::Closing>().prefs().drop_box {
                let _ = dropbox::show(tauri_app.handle(), true);
            }
            // The close button hides the window into the tray (`closing.rs`).
            closing::watch(tauri_app.handle());
            tauri_app.add_capability(windows::PROGRESS_CAPABILITY)?;
            tauri_app.add_capability(windows::SETTINGS_CAPABILITY)?;
            windows::close_with_main(tauri_app.handle());
            // The schedule window's and the «download complete» windows'
            // (`schedule.rs`, `complete.rs`); they close with the main one too.
            tauri_app.add_capability(schedule::CAPABILITY)?;
            tauri_app.add_capability(complete::CAPABILITY)?;
            #[cfg(feature = "ui-test")]
            if tauri_app.state::<ui_test::UiTest>().scenario.is_some() {
                // Fixed demo data: nothing runs, so nothing to wait for.
                tauri_app.state::<App>().stopped.send_replace(true);
                if let Some(window) = tauri_app.get_webview_window("main") {
                    // The add windows' scenes draw that window in the main
                    // one, at its own size.
                    let size = scene_size(tauri_app.handle());
                    ui_test::fit_to_screen(&window, size)?;
                }
                return Ok(());
            }
            // Is a newer Zajil out (#124)? In the background, once.
            update::check_at_start(&handle);
            tauri::async_runtime::spawn(async move {
                runner::run_queues(handle.clone()).await;
                handle.state::<App>().stopped.send_replace(true);
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("the window opens")
        .run(|handle, event| match event {
            // The last window went because the main one was freed in the
            // tray (`main_window.rs`): Zajil goes on. A quit has a code.
            tauri::RunEvent::ExitRequested {
                api, code: None, ..
            } if handle.state::<main_window::MainWindow>().freed() => {
                api.prevent_exit();
            }
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
                    shutdown::wait_for_queues(stopped, shutdown::STOP_WAIT).await;
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
                tauri::async_runtime::block_on(shutdown::wait_for_queues(
                    app.stopped.subscribe(),
                    shutdown::STOP_WAIT,
                ));
            }
            // macOS: a click on the Dock icon shows the window it hid.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => tray::show_window(handle),
            _ => {}
        });
}

// Generic over the runtime, so a test can run them on Tauri's mock runtime.
#[cfg(not(feature = "ui-test"))]
fn handlers<R: tauri::Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        downloads::snapshot,
        downloads::probe,
        downloads::guess_info,
        downloads::cancel_probe,
        downloads::add,
        downloads::pause,
        downloads::resume,
        downloads::remove,
        downloads::remove_with_files,
        downloads::details,
        tray::set_tray_texts,
        tray::set_tray_tooltip,
        closing::close_quits,
        closing::set_close_quits,
        sounds::sound_prefs,
        sounds::set_sound_prefs,
        addprefs::add_prefs,
        addprefs::set_add_prefs,
        addprefs::open_started_progress,
        downloads::set_item_details,
        video::window::video_pending,
        video::window::video_started,
        video::window::video_keep,
        video::window::video_discard,
        video::window::video_later,
        video::window::open_video,
        batch::open_batch,
        batch::batch_pending,
        batch::batch_preview,
        batch::batch_add,
        batch::clipboard_link,
        grab::open_grab,
        grab::grab_pending,
        grab::grab_check,
        grab::grab_cancel,
        grab::grab_add,
        video::window::video_folder,
        video::tools::video_tools_status,
        video::tools::video_tools_install,
        video::tools::video_tools_cancel,
        video::tools::video_tools_remove,
        video::queue::video_set_queue,
        video::probe::video_probe,
        video::probe::video_probe_cancel,
        video::window::video_window_open,
        video::window::show_video_window,
        video::download::video_download,
        video::download::video_add_many,
        video::download::video_cancel,
        dropbox::drop_box_shown,
        dropbox::set_drop_box,
        closing::quit_heard,
        settings::settings,
        settings::save_settings,
        settings::categories,
        settings::save_categories,
        settings::try_category,
        files::export_queues,
        files::import_queues,
        clipboard::show_window,
        integration::fix_browser_integration,
        integration::browser_integration,
        refresh::refresh_link,
        refresh::cancel_refresh,
        asks::go_without_cookies,
        windows::open_add_link,
        windows::pending_link,
        windows::open_add_info,
        background::hold_started,
        background::place_started,
        windows::pending_info,
        windows::window_role,
        measure::window_painted,
        windows::window_ready,
        windows::fit_window,
        offers::offer_answer,
        offers::add_offer,
        windows::close_window,
        windows::show_main_view,
        windows::open_progress,
        windows::open_settings,
        settings::proxy_setting,
        settings::set_proxy,
        settings::videos_at_once_setting,
        settings::set_videos_at_once,
        settings::finish_setting,
        settings::set_finish_setting,
        downloads::set_limit,
        menu::set_menu,
        closing::quit_app,
        menu::app_version,
        files::open_download,
        downloads::move_to_queue,
        clipboard::copy_links,
        schedule::open_schedule,
        schedule::queue_plans,
        schedule::new_queue,
        schedule::delete_queue,
        schedule::save_queue,
        schedule::move_in_queue,
        schedule::start_queue,
        schedule::set_after,
        schedule::countdown,
        schedule::cancel_after,
        schedule::choose_after_open,
        schedule::clear_after_open,
        complete::set_show_complete,
        complete::complete_info,
        update::update_notice,
        update::open_release_page,
        update::update_skip,
        update::update_install,
        update::extension_old_notice,
        update::open_extension_help,
        problem::copy_problem_info,
        complete::drag_out
    ]
}

#[cfg(feature = "ui-test")]
fn handlers<R: tauri::Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        downloads::snapshot,
        downloads::probe,
        downloads::guess_info,
        downloads::cancel_probe,
        downloads::add,
        downloads::pause,
        downloads::resume,
        downloads::remove,
        downloads::remove_with_files,
        downloads::details,
        tray::set_tray_texts,
        tray::set_tray_tooltip,
        closing::close_quits,
        closing::set_close_quits,
        sounds::sound_prefs,
        sounds::set_sound_prefs,
        addprefs::add_prefs,
        addprefs::set_add_prefs,
        addprefs::open_started_progress,
        downloads::set_item_details,
        video::window::video_pending,
        video::window::video_started,
        video::window::video_keep,
        video::window::video_discard,
        video::window::video_later,
        video::window::open_video,
        batch::open_batch,
        batch::batch_pending,
        batch::batch_preview,
        batch::batch_add,
        batch::clipboard_link,
        grab::open_grab,
        grab::grab_pending,
        grab::grab_check,
        grab::grab_cancel,
        grab::grab_add,
        video::window::video_folder,
        video::tools::video_tools_status,
        video::tools::video_tools_install,
        video::tools::video_tools_cancel,
        video::tools::video_tools_remove,
        video::queue::video_set_queue,
        video::probe::video_probe,
        video::probe::video_probe_cancel,
        video::window::video_window_open,
        video::window::show_video_window,
        video::download::video_download,
        video::download::video_add_many,
        video::download::video_cancel,
        dropbox::drop_box_shown,
        dropbox::set_drop_box,
        closing::quit_heard,
        settings::settings,
        settings::save_settings,
        settings::categories,
        settings::save_categories,
        settings::try_category,
        files::export_queues,
        files::import_queues,
        clipboard::show_window,
        integration::fix_browser_integration,
        integration::browser_integration,
        refresh::refresh_link,
        refresh::cancel_refresh,
        asks::go_without_cookies,
        windows::open_add_link,
        windows::pending_link,
        windows::open_add_info,
        background::hold_started,
        background::place_started,
        windows::pending_info,
        windows::window_role,
        measure::window_painted,
        windows::window_ready,
        windows::fit_window,
        offers::offer_answer,
        offers::add_offer,
        windows::close_window,
        windows::show_main_view,
        windows::open_progress,
        windows::open_settings,
        settings::proxy_setting,
        settings::set_proxy,
        settings::videos_at_once_setting,
        settings::set_videos_at_once,
        settings::finish_setting,
        settings::set_finish_setting,
        downloads::set_limit,
        menu::set_menu,
        closing::quit_app,
        menu::app_version,
        files::open_download,
        downloads::move_to_queue,
        clipboard::copy_links,
        schedule::open_schedule,
        schedule::queue_plans,
        schedule::new_queue,
        schedule::delete_queue,
        schedule::save_queue,
        schedule::move_in_queue,
        schedule::start_queue,
        schedule::set_after,
        schedule::countdown,
        schedule::cancel_after,
        schedule::choose_after_open,
        schedule::clear_after_open,
        complete::set_show_complete,
        complete::complete_info,
        update::update_notice,
        update::open_release_page,
        update::update_skip,
        update::update_install,
        update::extension_old_notice,
        update::open_extension_help,
        problem::copy_problem_info,
        complete::drag_out,
        ui_test::ui_test_scenario,
        ui_test::ui_test_report,
        ui_test::ui_test_e2e,
        ui_test::ui_test_step,
        ui_test::ui_test_windows,
        ui_test::ui_test_loaded
    ]
}

fn blocking_err(err: impl std::fmt::Display) -> String {
    err.to_string()
}
/// The size of the window a scene draws in the main one (logical pixels),
/// or `None` outside a scene or for a scene of the main window itself.
fn scene_size<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) -> Option<(f64, f64)> {
    #[cfg(feature = "ui-test")]
    {
        let scenario = handle.state::<ui_test::UiTest>().scenario.clone();
        let size = match scenario.as_deref().and_then(|s| s.split('-').next()) {
            Some("add") => Some(windows::INFO_SIZE),
            Some("link") => Some(windows::LINK_SIZE),
            Some("progress") => Some(windows::PROGRESS_SIZE),
            Some("tools" | "quality") => Some(video::window::SIZE),
            Some("complete") => Some(complete::SIZE),
            Some("schedule") => Some(schedule::SIZE),
            Some("batch") => Some(batch::SIZE),
            Some("grab") => Some(grab::SIZE),
            _ => None,
        };
        // macOS: the system's title bar is drawn by the page in a scene
        // (the main window's own is hidden, #118).
        size.map(|(w, h)| {
            (
                w,
                if cfg!(target_os = "macos") {
                    h + 28.0
                } else {
                    h
                },
            )
        })
    }
    #[cfg(not(feature = "ui-test"))]
    {
        let _ = handle;
        None
    }
}

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

/// The app on Tauri's mock runtime, for the tests: Zajil's own state and
/// commands on data folder `data`, and its main window, made from the
/// configuration as at the start.
#[cfg(test)]
pub(crate) fn mock_app(data: &Path) -> tauri::App<tauri::test::MockRuntime> {
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().app.windows = vec![tauri::utils::config::WindowConfig {
        label: main_window::LABEL.into(),
        ..Default::default()
    }];
    let app = managed(
        tauri::test::mock_builder(),
        new_app(data, data.join("Downloads")),
        data,
        false,
    )
    .invoke_handler(handlers())
    .build(context)
    .unwrap();
    #[cfg(feature = "ui-test")]
    app.manage(ui_test::UiTest::from_args(&[]));
    let config = app.config().app.windows[0].clone();
    tauri::WebviewWindowBuilder::from_config(&app, &config)
        .unwrap()
        .build()
        .unwrap();
    closing::watch(app.handle());
    windows::close_with_main(app.handle());
    app
}

/// Command `cmd` with `body`, called from `window`'s page, for the tests.
#[cfg(test)]
pub(crate) fn invoke_for_test(
    window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, serde_json::Value> {
    tauri::test::get_ipc_response(
        window,
        tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            // The page's own origin: anything else is refused.
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
    )
    .map(|answer| answer.deserialize().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #248: the start line said 0.0.0, this crate's version, whatever the
    /// installer's was.
    #[test]
    fn the_start_line_says_the_release_s_version() {
        let line = start_line(42);
        assert!(!line.contains("0.0.0"), "{line}");
        let release: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../scripts/release/tauri.release.json"
        ))
        .unwrap();
        let version = release["version"]
            .as_str()
            .expect("the installer's version");
        assert_eq!(
            line,
            format!("zajil: starting (version {version}, process 42)")
        );
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
        for cmd in [
            "ui_test_scenario",
            "ui_test_report",
            "ui_test_e2e",
            "ui_test_step",
            "ui_test_windows",
            "ui_test_loaded",
        ] {
            let body =
                serde_json::json!({ "report": {}, "step": "link", "detail": {}, "label": "main" });
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
}
