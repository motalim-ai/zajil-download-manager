//! The download «Download file info» starts at once, in the background
//! (the owner's request after trying other download managers): it runs
//! while the user picks the folder and the name. «Start download» moves
//! what is on disk to the chosen place and lets it run on; «Download later»
//! moves it and leaves it paused in the chosen queue; «Cancel», or closing
//! the window, takes it off the list and deletes its files.
//!
//! Until the window answers, the download is held here by the window's
//! label: a file that finishes meanwhile gets no «Download complete»
//! window yet (it shows once the file is where the user wants it).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{Emitter, Manager, State};
use zajil_core::queue::{QueueEvent, QueuesFile, Status, StoreError};

use crate::App;

/// How long a running download may take to stop before it moves anyway
/// (the runner looks twice a second; a slow disk saves its state late).
const STOP_WAIT: Duration = Duration::from_secs(30);

pub struct Held {
    /// The download each info window started, by the window's label.
    by_window: Mutex<HashMap<String, u64>>,
    /// The downloads that ended (stopped, finished or failed), as they end.
    ended: tokio::sync::broadcast::Sender<u64>,
}

impl Default for Held {
    fn default() -> Self {
        Held {
            by_window: Mutex::default(),
            ended: tokio::sync::broadcast::channel(64).0,
        }
    }
}

impl Held {
    /// Whether an info window still holds download `id`.
    pub fn holds(&self, id: u64) -> bool {
        self.by_window.lock().unwrap().values().any(|i| *i == id)
    }

    fn take(&self, label: &str) -> Option<u64> {
        self.by_window.lock().unwrap().remove(label)
    }
}

/// The queues' events: who waits for a download to end hears of it.
pub fn on_event(handle: &tauri::AppHandle, event: &QueueEvent) {
    let id = match event {
        QueueEvent::Stopped { item }
        | QueueEvent::Finished { item, .. }
        | QueueEvent::Failed { item, .. }
        | QueueEvent::Retrying { item, .. } => *item,
        _ => return,
    };
    let _ = handle.state::<Held>().ended.send(id);
}

/// The info window that asks started download `id` in the background.
#[tauri::command]
pub fn hold_started<R: tauri::Runtime>(
    held: State<'_, Held>,
    id: u64,
    window: tauri::WebviewWindow<R>,
) {
    let label = crate::windows::role_of(&window);
    held.by_window.lock().unwrap().insert(label, id);
}

/// «Start download» or «Download later» on the download the window that
/// asks started: it goes to `folder` (`None`: where it is) under `name`
/// (`None`: its name), then runs on when `start`, or waits paused.
#[tauri::command]
pub async fn place_started<R: tauri::Runtime>(
    app: State<'_, App>,
    held: State<'_, Held>,
    folder: Option<String>,
    name: Option<String>,
    start: bool,
    window: tauri::WebviewWindow<R>,
    handle: tauri::AppHandle<R>,
) -> Result<u64, String> {
    let label = crate::windows::role_of(&window);
    let id = *held
        .by_window
        .lock()
        .unwrap()
        .get(&label)
        .ok_or("this window started no download")?;
    let result = place(&app.store, &held, id, folder, name, start).await;
    let _ = handle.emit("downloads-changed", ());
    // Released only once it is placed: closing the window on an error
    // still deletes it.
    result?;
    held.take(&label);
    // Finished while the window was open: its window shows now.
    crate::complete::show_if_done(handle.app_handle(), id);
    Ok(id)
}

async fn place(
    store: &zajil_core::queue::QueueStore,
    held: &Held,
    id: u64,
    folder: Option<String>,
    name: Option<String>,
    start: bool,
) -> Result<(), String> {
    let mut ended = held.ended.subscribe();
    // Paused first: nothing writes to its files while they move.
    let was = blocking(store, move |q| {
        let item = q.item_mut(id).ok_or(StoreError::NoSuchItem(id))?;
        let was = item.status;
        if matches!(was, Status::Active | Status::Waiting) {
            item.status = Status::Paused;
        }
        Ok(was)
    })
    .await?;
    if was == Status::Active {
        let _ = tokio::time::timeout(STOP_WAIT, async {
            loop {
                match ended.recv().await {
                    Ok(item) if item == id => return,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    _ => {}
                }
            }
        })
        .await;
    }
    let folder = match folder {
        Some(folder) => Some(crate::chosen_folder(Some(&folder), Path::new(""))?),
        None => None,
    };
    let store = store.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let queues = store.load().map_err(|e| e.to_string())?;
        let item = queues
            .item(id)
            .ok_or(StoreError::NoSuchItem(id).to_string())?;
        let moved = moved(item, folder.as_deref(), name.as_deref())?;
        store
            .update(move |q| {
                let item = q.item_mut(id).ok_or(StoreError::NoSuchItem(id))?;
                apply(item, moved, start);
                Ok(())
            })
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Where a download is after the move.
enum Moved {
    /// Not begun yet: it starts there, under that name.
    NotBegun {
        dir: Option<PathBuf>,
        name: Option<String>,
    },
    Unfinished(PathBuf),
    Finished(PathBuf),
}

/// Moves `item`'s files to `folder` under `name` (each `None`: as it is).
fn moved(
    item: &zajil_core::queue::Item,
    folder: Option<&Path>,
    name: Option<&str>,
) -> Result<Moved, String> {
    let target = |now: &Path| {
        let dir = folder
            .map(Path::to_path_buf)
            .or_else(|| now.parent().map(Path::to_path_buf))
            .unwrap_or_default();
        let name = name.map(str::to_owned).unwrap_or_else(|| {
            now.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        (dir, name)
    };
    if let (Status::Done, Some(file)) = (item.status, &item.file) {
        let (dir, name) = target(file);
        return zajil_core::relocate::move_finished(file, &dir, &name)
            .map(Moved::Finished)
            .map_err(|e| e.to_string());
    }
    if let Some(state) = &item.state_file
        && let Some(now) = zajil_core::state::final_path_for_state(state)
    {
        let (dir, name) = target(&now);
        return zajil_core::relocate::move_unfinished(state, &dir, &name)
            .map(Moved::Unfinished)
            .map_err(|e| e.to_string());
    }
    Ok(Moved::NotBegun {
        dir: folder.map(Path::to_path_buf),
        name: name.map(str::to_owned),
    })
}

fn apply(item: &mut zajil_core::queue::Item, moved: Moved, start: bool) {
    match moved {
        Moved::NotBegun { dir, name } => {
            if let Some(dir) = dir {
                item.dir = dir;
                item.categorize = false;
            }
            if name.is_some() {
                item.name = name.map(|n| zajil_core::sanitize::sanitize_file_name(&n));
            }
        }
        Moved::Unfinished(state) => {
            if let Some(dir) = state.parent() {
                item.dir = dir.to_path_buf();
            }
            item.categorize = false;
            item.state_file = Some(state);
        }
        Moved::Finished(file) => {
            if let Some(dir) = file.parent() {
                item.dir = dir.to_path_buf();
            }
            item.file = Some(file);
        }
    }
    if start && item.status == Status::Paused {
        item.status = Status::Waiting;
        item.error = None;
    }
}

/// The info window `label` closed: a download it started and did not
/// place is taken off the list with its files (as «Cancel»), even one
/// that finished meanwhile.
pub fn window_gone<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, label: &str) {
    let Some(id) = handle.state::<Held>().take(label) else {
        return;
    };
    let store = handle.state::<App>().store.clone();
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let gone = tokio::task::spawn_blocking(move || {
            let queues = store.load()?;
            let mut files = crate::removal::files_of(&queues, &[id]);
            if let Some(file) = queues.item(id).and_then(|i| i.file.clone()) {
                files.push(file);
            }
            store.update(|q| q.remove(id).map(|_| ()))?;
            Ok::<_, StoreError>(files)
        })
        .await;
        if let Ok(Ok(files)) = gone {
            crate::removal::delete_soon(files);
        }
        let _ = handle.emit("downloads-changed", ());
    });
}

async fn blocking<T: Send + 'static>(
    store: &zajil_core::queue::QueueStore,
    change: impl FnOnce(&mut QueuesFile) -> Result<T, StoreError> + Send + 'static,
) -> Result<T, String> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || store.update(change))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zajil_core::queue::QueuesFile;

    fn item(dir: &Path) -> (QueuesFile, u64) {
        let mut q = QueuesFile::empty();
        let id = q.add("main", "https://example.com/a.bin", dir).unwrap();
        (q, id)
    }

    /// Not begun yet (its queue was busy): it simply starts in the chosen
    /// folder, under the chosen name, and is no longer put by category.
    #[test]
    fn a_download_not_begun_takes_the_folder_and_name() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut q, id) = item(tmp.path());
        let item = q.item_mut(id).unwrap();
        item.categorize = true;
        item.status = Status::Paused;
        let other = tmp.path().join("other");
        let moved = moved(item, Some(&other), Some("b/../c.bin")).unwrap();
        apply(item, moved, true);
        assert_eq!(item.dir, other);
        assert!(!item.categorize);
        let name = item.name.as_deref().unwrap();
        assert!(
            name.ends_with("c.bin") && !name.contains(['/', '\\']),
            "{name}"
        );
        assert_eq!(item.status, Status::Waiting);
    }

    /// The whole way, with the queues' runner: a download running from a
    /// local server is moved to another folder and name while it runs,
    /// and finishes there with all its bytes, none asked for twice.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_running_download_moves_and_finishes_where_the_user_said() {
        use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};
        let server = TestServer::http();
        let data = random_bytes(6 * 1024 * 1024, 5);
        server.put("/f.bin", FileSpec::new(data.clone()).throttle(1024 * 1024));
        let tmp = tempfile::tempdir().unwrap();
        let (first, chosen) = (tmp.path().join("first"), tmp.path().join("chosen"));
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&chosen).unwrap();
        let store = zajil_core::queue::QueueStore::new(tmp.path());
        let url = server.url("/f.bin");
        let id = store.update(|q| q.add("main", &url, &first)).unwrap();
        let held = Held::default();
        let ended = held.ended.clone();
        let stop = zajil_core::Stop::new();
        let runner = zajil_core::queue::Runner::new(
            zajil_core::Engine::new(zajil_core::Options {
                connections: 2,
                ..zajil_core::Options::default()
            })
            .unwrap(),
            store.clone(),
        )
        .poll(Duration::from_millis(50));
        let running = {
            let stop = stop.clone();
            // Run again when something waits, as the app does
            // (`run_queues`): a runner ends when nothing is left to run.
            tokio::spawn(async move {
                while !stop.is_requested() {
                    let _ = runner
                        .run(&stop, &mut |event| {
                            if let QueueEvent::Stopped { item }
                            | QueueEvent::Finished { item, .. }
                            | QueueEvent::Failed { item, .. } = event
                            {
                                let _ = ended.send(item);
                            }
                        })
                        .await;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
        };
        let item = |store: &zajil_core::queue::QueueStore| {
            store.load().unwrap().item(id).cloned().unwrap()
        };
        // Running, with some of it on disk.
        let mut waited = 0;
        while item(&store).state_file.is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
            waited += 1;
            assert!(waited < 500, "the download never started");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(item(&store).status, Status::Active);
        place(
            &store,
            &held,
            id,
            Some(chosen.to_string_lossy().into_owned()),
            Some("الملف الجديد.bin".into()),
            true,
        )
        .await
        .unwrap();
        let mut waited = 0;
        while item(&store).status != Status::Done {
            tokio::time::sleep(Duration::from_millis(50)).await;
            waited += 1;
            assert!(
                waited < 600,
                "the download never finished: {:?}",
                item(&store)
            );
        }
        stop.request();
        let _ = running.await;
        let file = item(&store).file.unwrap();
        assert_eq!(file.file_name().unwrap(), "الملف الجديد.bin");
        assert_eq!(file.parent().unwrap().file_name(), chosen.file_name());
        assert_eq!(
            sha256_hex(&std::fs::read(&file).unwrap()),
            sha256_hex(&data)
        );
        assert_eq!(std::fs::read_dir(&first).unwrap().count(), 0);
        // Resumed from where it was: the bytes sent add up to the file once.
        let sent: u64 = server.gets("/f.bin").iter().map(|g| g.bytes_sent).sum();
        assert!(sent < data.len() as u64 + 1024 * 1024, "{sent}");
    }

    /// Paused part-way: its files move, and it waits paused for «Download
    /// later», or runs on for «Start download».
    #[test]
    fn an_unfinished_download_moves_with_its_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let other = root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        // As the engine spells it (the folder resolved).
        std::fs::write(root.join("a.bin"), b"").unwrap();
        let final_path = zajil_core::sanitize::confine(&root, "a.bin").unwrap();
        std::fs::remove_file(&final_path).unwrap();
        std::fs::write(zajil_core::state::part_path(&final_path), b"half").unwrap();
        let mut state = zajil_core::state::DownloadState::new(
            "https://example.com/a.bin".into(),
            "https://example.com/a.bin".into(),
            Default::default(),
            Some(8),
            true,
        );
        state.segments[0].done = 4;
        let state_path = zajil_core::state::state_path(&final_path);
        state.save(&state_path).unwrap();
        for start in [false, true] {
            let (mut q, id) = item(&root);
            let item = q.item_mut(id).unwrap();
            item.status = Status::Paused;
            item.state_file = Some(state_path.clone());
            let name = if start { "a.bin" } else { "b.bin" };
            let folder = if start { &root } else { &other };
            let moved = moved(item, Some(folder), Some(name)).unwrap();
            apply(item, moved, start);
            let now = item.state_file.clone().unwrap();
            // Where it was asked to go (by name: Windows may spell the
            // folder another way).
            assert!(now.exists());
            assert_eq!(
                now.file_name().unwrap(),
                zajil_core::state::state_path(&folder.join(name))
                    .file_name()
                    .unwrap()
            );
            assert_eq!(now.parent().unwrap().file_name(), folder.file_name());
            assert_eq!(
                item.status,
                if start {
                    Status::Waiting
                } else {
                    Status::Paused
                }
            );
            // Back where it was, for the second turn.
            if !start {
                std::fs::rename(&now, &state_path).unwrap();
                std::fs::rename(
                    zajil_core::state::part_path(&other.join("b.bin")),
                    zajil_core::state::part_path(&final_path),
                )
                .unwrap();
            }
        }
    }
}
