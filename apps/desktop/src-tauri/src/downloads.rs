//! The list's commands: what the page shows, adding a download, and
//! changing the downloads in the queues file.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use tauri::{Emitter, Manager, State};
use zajil_core::Stop;
use zajil_core::category::{CategoriesFile, CategoryStore};
use zajil_core::queue::{DEFAULT_QUEUE, QueueStore, QueuesFile, Status, StoreError};
use zajil_core::settings::SettingsStore;
use zajil_core::state::DownloadState;

use crate::asks::send_keep;
use crate::live::Live;
#[cfg(feature = "ui-test")]
use crate::ui_test;
use crate::view::{self, AddRequest, DetailsView, ProbeView, Snapshot};
use crate::{App, background, blocking_err, measure, removal, video, windows};

#[tauri::command]
pub(crate) async fn snapshot<R: tauri::Runtime>(
    app: State<'_, App>,
    handle: tauri::AppHandle<R>,
    window: tauri::WebviewWindow<R>,
) -> Result<Snapshot, String> {
    // The main page's first look: other windows read the list too (the
    // schedule's, «Download file info»), and are not it.
    let main = window.label() == crate::main_window::LABEL;
    if main && !app.page_ready.swap(true, Ordering::SeqCst) {
        // The page is up and talking to the program (the open-and-close
        // check of CI waits for this line).
        eprintln!("zajil: the window is ready");
        // The windows the user waits for are made now, hidden.
        windows::warm(&handle);
        if measure::on() || measure::show().is_some() || measure::tray() {
            measure::drive(&handle);
        }
    }
    // The first look of a main page made again: what Zajil said in the
    // tray while there was none, it says from now (`in_tray.rs`).
    if main {
        crate::in_tray::hand_over(&handle).await;
    }
    #[cfg(feature = "ui-test")]
    if let Some(snapshot) = handle.state::<ui_test::UiTest>().demo() {
        return Ok(snapshot);
    }
    listed(&app, &handle).await
}

/// What the list shows: the queues' downloads, then the videos, without
/// those a window still asks about. The page's, and Zajil's own while the
/// main window is freed in the tray (`in_tray.rs`).
pub(crate) async fn listed<R: tauri::Runtime>(
    app: &App,
    handle: &tauri::AppHandle<R>,
) -> Result<Snapshot, String> {
    let (store, categories, live) = (app.store.clone(), app.categories.clone(), app.live.clone());
    let mut snapshot =
        tokio::task::spawn_blocking(move || read_snapshot(&store, &categories, &live))
            .await
            .map_err(blocking_err)??;
    // The video downloads, after the queues' (#149).
    let categories = view::categories_from(&snapshot.categories);
    let categories = CategoriesFile {
        categories,
        ..no_categories()
    };
    snapshot.downloads.extend(video::views(
        &handle.state::<video::VideoState>(),
        &categories,
    ));
    // A download an info window or «Video quality» still asks about is not
    // in the list before «Start download» (as IDM: it runs unseen).
    let held = handle.state::<background::Held>();
    let asked = video::asked(&handle.state::<video::VideoState>());
    snapshot
        .downloads
        .retain(|d| !held.holds(d.id) && Some(d.id) != asked);
    Ok(snapshot)
}

#[tauri::command]
pub(crate) async fn probe(app: State<'_, App>, url: String) -> Result<ProbeView, String> {
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

/// What «Download file info» shows before the server answers: the name the
/// link's address gives, its category by that name and the site, and the
/// folder it would go to, with no size. The window is drawn whole from its
/// first frame, and the server's answer only fills it in (the owner's
/// notes: it opened small and grew once the server answered).
#[tauri::command]
pub(crate) async fn guess_info(app: State<'_, App>, url: String) -> Result<ProbeView, String> {
    let found = guessed(&url)?;
    let (categories, settings, system) = (
        app.categories.clone(),
        app.settings.clone(),
        app.system_download_dir.clone(),
    );
    tokio::task::spawn_blocking(move || {
        let categories = categories.load().unwrap_or_else(|_| no_categories());
        let given = download_dir(&settings, &system);
        view::probe(&found, &categories, &given)
    })
    .await
    .map_err(blocking_err)
}

/// What `url` says of its file by itself ([`guess_info`]).
fn guessed(url: &str) -> Result<zajil_core::Probe, String> {
    let parsed = zajil_core::Url::parse(url.trim()).map_err(|e| e.to_string())?;
    Ok(zajil_core::Probe {
        name: zajil_core::naming::suggested_name(None, &parsed),
        size: None,
        resumable: false,
        ranges: false,
        host: parsed.host_str().map(str::to_owned),
        content_type: None,
    })
}

/// Gives up the add dialog's probe (the dialog was closed).
#[tauri::command]
pub(crate) fn cancel_probe(app: State<'_, App>) {
    if let Some(stop) = app.probing.lock().unwrap().take() {
        stop.request();
    }
}

/// What a damaged categories file is read as: no categories at all (the
/// list says the problem itself).
pub(crate) fn no_categories() -> CategoriesFile {
    let mut none = CategoriesFile::built_in();
    none.categories.clear();
    none
}

#[tauri::command]
pub(crate) async fn add<R: tauri::Runtime>(
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
        add_blocking(&store, &request, &given, None, false, false, false, false)
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
pub(crate) fn chosen_folder(folder: Option<&str>, download_dir: &Path) -> Result<PathBuf, String> {
    match folder {
        Some(folder) => {
            let folder = PathBuf::from(folder);
            zajil_core::category::check_folder(&folder).map_err(|e| e.to_string())?;
            Ok(folder)
        }
        None => Ok(download_dir.to_path_buf()),
    }
}

/// Nothing is created here: the download makes its folder when it starts,
/// so a refused add leaves nothing behind (review of #48).
#[allow(clippy::too_many_arguments)]
pub(crate) fn add_blocking(
    store: &QueueStore,
    request: &AddRequest,
    given: &Path,
    browser: Option<zajil_core::queue::BrowserSource>,
    needs_ok: bool,
    ok_given: bool,
    from_page: bool,
    private: bool,
) -> Result<u64, StoreError> {
    store.update(|queues| {
        let queue = request.queue.as_deref().unwrap_or(DEFAULT_QUEUE);
        let id = queues.add(queue, &request.url, given)?;
        // The user's name, category and description (#103, #95), in the
        // same write: the queue never sees the download without them.
        let given_name = request
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty());
        if given_name.is_some() {
            queues.set_name(id, given_name)?;
        }
        if let Some(category) = request.category.as_deref() {
            queues.set_category(id, Some(category))?;
        }
        let description = request
            .description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty());
        if description.is_some() {
            queues.set_description(id, description)?;
        }
        if let Some(n) = request.connections {
            queues.set_connections(id, Some(n))?;
        }
        let item = queues.item_mut(id).expect("just added");
        // Without a folder of its own, a download goes by its category.
        item.categorize = request.folder.is_none();
        item.limit = request.limit.filter(|l| *l > 0);
        item.browser = browser;
        item.private = private;
        if !request.start {
            item.status = Status::Paused;
        }
        // In the same write: the queue never sees it waiting.
        item.needs_ok = needs_ok;
        item.ok_given = ok_given;
        item.from_page = from_page;
        Ok(id)
    })
}

/// What the page shows, read from the files. Blocking. A damaged categories
/// file is said, not fatal: the downloads still show (review of #45: the
/// whole list went blank while they ran).
/// A file as it is on disk now: where, its size, and when it was made and
/// last written (each change replaces the file, ADR-0004).
type Stamp = (
    PathBuf,
    u64,
    Option<std::time::SystemTime>,
    Option<std::time::SystemTime>,
);

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = path.symlink_metadata().ok()?;
    Some((
        path.to_path_buf(),
        meta.len(),
        meta.created().ok(),
        meta.modified().ok(),
    ))
}

/// `load`'s answer, read again only when the file at `path` changed: while
/// downloads run, the list asks four times a second, and their progress
/// is in memory, not in the files (the owner's request: a faster list).
fn cached<T: Clone, E>(
    cell: &std::sync::Mutex<Option<(Stamp, T)>>,
    path: &Path,
    load: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let now = stamp(path);
    if let Some(now) = &now
        && let Some((was, value)) = &*cell.lock().unwrap_or_else(|e| e.into_inner())
        && was == now
    {
        return Ok(value.clone());
    }
    let value = load()?;
    // Stamped before the read: a change during it is read next time.
    *cell.lock().unwrap_or_else(|e| e.into_inner()) = now.map(|now| (now, value.clone()));
    Ok(value)
}

static QUEUES_READ: std::sync::Mutex<Option<(Stamp, QueuesFile)>> = std::sync::Mutex::new(None);
static CATEGORIES_READ: std::sync::Mutex<Option<(Stamp, CategoriesFile)>> =
    std::sync::Mutex::new(None);

pub(crate) fn read_snapshot(
    store: &QueueStore,
    categories: &CategoryStore,
    live: &Live,
) -> Result<Snapshot, String> {
    let queues = cached(&QUEUES_READ, &store.path(), || store.load()).map_err(blocking_err)?;
    let (categories, problem) =
        match cached(&CATEGORIES_READ, &categories.path(), || categories.load()) {
            Ok(c) => (c, None),
            Err(err) => {
                let mut none = CategoriesFile::built_in();
                none.categories.clear();
                (none, Some(err.to_string()))
            }
        };
    let mut snapshot = view::snapshot(&queues, &categories, live);
    snapshot.categories_problem = problem.clone();
    snapshot.problem = snapshot.problem.or(problem);
    Ok(snapshot)
}

/// Sets the status of items, as `zajil queue pause` and `resume` do: an
/// unknown id is an error and changes nothing. A running download stops
/// after recording its progress (the runner sees the change within half a
/// second).
pub(crate) fn set_status(
    queues: &mut QueuesFile,
    ids: &[u64],
    status: Status,
) -> Result<(), StoreError> {
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
                // Started by the user: its queue's tries count from 0
                // (#142).
                item.failed_tries = 0;
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

pub(crate) async fn change<R: tauri::Runtime>(
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
pub(crate) async fn pause<R: tauri::Runtime>(
    app: State<'_, App>,
    ids: Vec<u64>,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    let (videos, ids) = video::split(ids);
    video::pause(&handle.state::<video::VideoState>(), &videos);
    change(&app, &handle, move |q| set_status(q, &ids, Status::Paused)).await
}

#[tauri::command]
pub(crate) async fn resume<R: tauri::Runtime>(
    app: State<'_, App>,
    ids: Vec<u64>,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    // A download whose link is being refreshed waits for the refresh: it
    // would otherwise start from the old link, or start over.
    let (videos, ids) = video::split(ids);
    video::resume(&handle, &videos);
    let ids = app.refreshing.without(ids);
    change(&app, &handle, move |q| set_status(q, &ids, Status::Waiting)).await
}

/// A download's own speed limit (the progress window, M5.5); `None` or 0:
/// none. A running download takes it at once: the runner gives each
/// running download its item's limit whenever the queues file changes
/// (#115).
#[tauri::command]
pub(crate) async fn set_limit<R: tauri::Runtime>(
    app: State<'_, App>,
    id: u64,
    limit: Option<u64>,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    change(&app, &handle, move |q| {
        q.item_mut(id).ok_or(StoreError::NoSuchItem(id))?.limit = limit.filter(|l| *l > 0);
        Ok(())
    })
    .await
}

/// Moves downloads to queue `queue`, in their order, at its end.
#[tauri::command]
pub(crate) async fn move_to_queue<R: tauri::Runtime>(
    app: State<'_, App>,
    ids: Vec<u64>,
    queue: String,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    // Videos join a queue as well (owner's request, 2026-10-30).
    let (videos, ids) = video::split(ids);
    if !videos.is_empty() {
        video::set_queue(&handle, &videos, Some(queue.clone()));
    }
    if ids.is_empty() {
        return Ok(());
    }
    change(&app, &handle, move |q| move_items(q, &ids, &queue)).await
}

/// [`move_to_queue`]: an unknown download or queue changes nothing.
fn move_items(queues: &mut QueuesFile, ids: &[u64], queue: &str) -> Result<(), StoreError> {
    queues
        .queue(queue)
        .ok_or_else(|| StoreError::NoSuchQueue(queue.to_owned()))?;
    for id in ids {
        queues.item(*id).ok_or(StoreError::NoSuchItem(*id))?;
    }
    for id in ids {
        let item = queues.remove(*id)?;
        queues.queue_mut(queue).expect("checked").items.push(item);
    }
    Ok(())
}

/// Takes downloads off the list and deletes their unfinished files (the
/// part and state files; a finished file stays): deleting downloads that
/// have not finished, when the user asks for their files too.
#[tauri::command]
pub(crate) async fn remove_with_files<R: tauri::Runtime>(
    app: State<'_, App>,
    ids: Vec<u64>,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    let (videos, ids) = video::split(ids);
    video::remove_with_files(&handle, &videos);
    let store = app.store.clone();
    let files = {
        let ids = ids.clone();
        tokio::task::spawn_blocking(move || store.load().map(|q| removal::files_of(&q, &ids)))
            .await
            .map_err(blocking_err)?
            .map_err(|e| e.to_string())?
    };
    change(&app, &handle, move |q| remove_items(q, &ids)).await?;
    send_keep(&app.links, &app.store, None);
    removal::delete_soon(files);
    Ok(())
}

/// A download's category and description, set after it was added (a
/// download started while «Download file info» was open).
#[tauri::command]
pub(crate) async fn set_item_details<R: tauri::Runtime>(
    app: State<'_, App>,
    id: u64,
    category: Option<String>,
    description: Option<String>,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    change(&app, &handle, move |q| {
        if category.is_some() {
            q.set_category(id, category.as_deref())?;
        }
        q.set_description(
            id,
            description
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty()),
        )
    })
    .await
}

#[tauri::command]
pub(crate) async fn remove<R: tauri::Runtime>(
    app: State<'_, App>,
    ids: Vec<u64>,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    let (videos, ids) = video::split(ids);
    video::remove(&handle, &videos);
    change(&app, &handle, move |q| remove_items(q, &ids)).await?;
    // Removed downloads: their browsers forget their tickets at once.
    send_keep(&app.links, &app.store, None);
    Ok(())
}

#[tauri::command]
pub(crate) async fn details<R: tauri::Runtime>(
    app: State<'_, App>,
    id: u64,
    handle: tauri::AppHandle<R>,
) -> Result<DetailsView, String> {
    // The demo download's details only in a scene (review of #47).
    #[cfg(feature = "ui-test")]
    if handle.state::<ui_test::UiTest>().scenario.is_some() {
        return ui_test::demo_details(id).ok_or_else(|| format!("no download {id}"));
    }
    let (store, categories, live) = (app.store.clone(), app.categories.clone(), app.live.clone());
    if video::is_video(id) {
        let categories = categories.load().unwrap_or_else(|_| no_categories());
        return video::details(&handle.state::<video::VideoState>(), &categories, id)
            .ok_or_else(|| format!("no download {id}"));
    }
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
pub(crate) fn download_dir(settings: &SettingsStore, system: &Path) -> PathBuf {
    settings
        .load()
        .ok()
        .and_then(|s| s.download_dir)
        .unwrap_or_else(|| system.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// «Download file info» draws its rows before the server answers with
    /// what the address says: the file's name (decoded and made safe), its
    /// site for the categories, and no size.
    #[test]
    fn the_info_window_guesses_from_the_address() {
        let found =
            guessed(" https://files.example.com/a/%D9%85%D9%84%D9%81%20v2.zip?x=1 ").unwrap();
        assert_eq!(found.name, "ملف v2.zip");
        assert_eq!(found.host.as_deref(), Some("files.example.com"));
        assert_eq!(
            (found.size, found.resumable, found.is_page()),
            (None, false, false)
        );
        assert_eq!(guessed("https://example.com/").unwrap().name, "download");
        assert_eq!(
            guessed("https://example.com/../../etc/passwd")
                .unwrap()
                .name,
            "passwd"
        );
        assert!(guessed("not a link").is_err());
    }

    /// «Download file info» adds the download with the user's own name,
    /// category and description (#103, #95); an empty name or description
    /// leaves the server's name and none.
    #[test]
    fn a_download_is_added_with_the_users_name_category_and_description() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let request = AddRequest {
            url: "https://example.com/v.mp4".into(),
            name: Some("  درس أول.mp4 ".into()),
            category: Some("music".into()),
            description: Some(" للمراجعة ".into()),
            queue: None,
            folder: None,
            start: false,
            limit: None,
            connections: None,
        };
        let id = add_blocking(
            &store,
            &request,
            data.path(),
            None,
            false,
            false,
            false,
            false,
        )
        .unwrap();
        let queues = store.load().unwrap();
        let item = queues.item(id).unwrap();
        assert_eq!(item.name.as_deref(), Some("درس أول.mp4"));
        assert_eq!(item.category.as_deref(), Some("music"));
        assert_eq!(item.description.as_deref(), Some("للمراجعة"));

        let plain = AddRequest {
            name: Some("   ".into()),
            category: None,
            description: Some("".into()),
            ..request
        };
        let id = add_blocking(
            &store,
            &plain,
            data.path(),
            None,
            false,
            false,
            false,
            false,
        )
        .unwrap();
        let queues = store.load().unwrap();
        let item = queues.item(id).unwrap();
        assert_eq!(
            (
                item.name.as_ref(),
                item.category.as_ref(),
                item.description.as_ref()
            ),
            (None, None, None)
        );
    }

    fn three() -> QueuesFile {
        let mut q = QueuesFile::empty();
        let dir = std::env::temp_dir();
        for n in 1..=3 {
            q.add("main", &format!("https://example.com/{n}.iso"), &dir)
                .unwrap();
        }
        q
    }

    /// M5.5 «Move to queue»: the downloads go to the end of that queue, in
    /// their order; an unknown queue or download changes nothing.
    #[test]
    fn downloads_move_to_another_queue_in_their_order() {
        let mut q = three();
        q.add("night", "https://example.com/n.iso", &std::env::temp_dir())
            .unwrap();
        let before = q.clone();
        assert!(matches!(
            move_items(&mut q, &[1], "nowhere"),
            Err(StoreError::NoSuchQueue(_))
        ));
        assert!(matches!(
            move_items(&mut q, &[1, 99], "night"),
            Err(StoreError::NoSuchItem(99))
        ));
        assert_eq!(q, before);
        move_items(&mut q, &[3, 1], "night").unwrap();
        let ids = |name: &str| -> Vec<u64> {
            q.queue(name).unwrap().items.iter().map(|i| i.id).collect()
        };
        assert_eq!(ids("night"), [4, 3, 1]);
        assert_eq!(ids("main"), [2]);
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
        q.item_mut(2).unwrap().failed_tries = 2;
        set_status(&mut q, &[1, 2], Status::Waiting).unwrap();
        assert_eq!(q.item(1).unwrap().status, Status::Waiting);
        assert_eq!(q.item(2).unwrap().status, Status::Waiting);
        assert!(q.item(2).unwrap().error.is_none());
        // Resumed by the user: its queue's tries count from 0 again (#142).
        assert_eq!(q.item(2).unwrap().failed_tries, 0);
    }

    /// #58: a categories file that cannot be read says why on the
    /// categories screen (it showed an empty list with no reason), apart
    /// from the queues' own problem.
    #[test]
    fn a_damaged_categories_file_says_why() {
        let data = tempfile::tempdir().unwrap();
        let (store, categories, live) = (
            QueueStore::new(data.path()),
            CategoryStore::new(data.path()),
            Live::default(),
        );
        let fine = read_snapshot(&store, &categories, &live).unwrap();
        assert_eq!(fine.categories_problem, None);
        assert!(!fine.categories.is_empty());
        std::fs::write(categories.path(), b"not a categories file").unwrap();
        let damaged = read_snapshot(&store, &categories, &live).unwrap();
        assert!(damaged.categories.is_empty());
        assert!(damaged.categories_problem.is_some());
        assert_eq!(damaged.problem, damaged.categories_problem);
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
