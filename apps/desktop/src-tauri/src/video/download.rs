//! A video's download: started from «Video quality» or a playlist, run,
//! and its progress told to the list.

use std::sync::atomic::Ordering;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};
use ts_rs::TS;
use zajil_core::Stop;

use super::choice::{from_list, still_offered};
use super::probe::{ahead_fits, probe};
use super::queue::{QUEUE_VIDEO_DONE, known_queue, pump};
use super::rows::{key_of, remove_leftovers, save_rows, with_row};
use super::views::{ChoiceView, VideoErrorView};
use super::{FIRST_ID, Row, VideoState, runner};
use crate::view::StatusView;
use zajil_core::video::{Download, Probed, VideoEvent};

#[derive(Debug, Clone, Serialize)]
struct Progress {
    id: u64,
    done: u64,
    total: Option<u64>,
    speed: Option<u64>,
}

/// Downloads a probed video into `dest` as `name`: it joins the list, and
/// its id comes at once; then `video-progress`, and `video-finished` or
/// `video-failed`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn video_download<R: tauri::Runtime>(
    app: State<'_, crate::App>,
    state: State<'_, VideoState>,
    url: String,
    choice: ChoiceView,
    dest: String,
    name: String,
    queue: Option<String>,
    handle: tauri::AppHandle<R>,
) -> Result<u64, String> {
    let queue = known_queue(&app, queue);
    let id = FIRST_ID + state.next.fetch_add(1, Ordering::SeqCst) + 1;
    let private = state.private.lock().unwrap().contains(&url);
    let url = state
        .read_as
        .lock()
        .unwrap()
        .get(&url)
        .cloned()
        .unwrap_or(url);
    state.rows.lock().unwrap().push(Row {
        id,
        url,
        name,
        folder: dest,
        choice,
        status: StatusView::Waiting,
        size: None,
        done: 0,
        speed: 0,
        error: None,
        file: None,
        private,
        move_to: None,
        label: None,
        added: Some(zajil_core::state::now_rfc3339()),
        queue: None,
        agents: Vec::new(),
    });
    with_row(&state, id, |row| row.queue = queue);
    state.fresh.lock().unwrap().insert(id);
    save_rows(&app, &state);
    let _ = handle.emit("downloads-changed", ());
    pump(&handle);
    Ok(id)
}

/// A video of a playlist, for [`video_add_many`].
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct NewVideo {
    pub url: String,
    pub title: Option<String>,
}

/// A playlist's chosen videos join the list at once, each at its best,
/// named after its title; they start in turn, at most the settings' number at
/// a time, and each asks its own page then. Those on the playlist's site
/// take its cookies, if the browser sent any. Returns their ids.
#[tauri::command]
pub fn video_add_many<R: tauri::Runtime>(
    app: State<'_, crate::App>,
    state: State<'_, VideoState>,
    page: String,
    items: Vec<NewVideo>,
    dest: String,
    queue: Option<String>,
    handle: tauri::AppHandle<R>,
) -> Vec<u64> {
    let queue = known_queue(&app, queue);
    let page_cookies = state.cookies.lock().unwrap().get(&page).cloned();
    let private = state.private.lock().unwrap().contains(&page);
    let mut ids = Vec::new();
    for item in items {
        if let Some(cookies) = &page_cookies
            && same_site(&page, &item.url)
        {
            state
                .cookies
                .lock()
                .unwrap()
                .entry(item.url.clone())
                .or_insert_with(|| cookies.clone());
        }
        let id = FIRST_ID + state.next.fetch_add(1, Ordering::SeqCst) + 1;
        let mut row = new_row(id, item, &dest);
        row.private = private;
        row.queue = queue.clone();
        state.rows.lock().unwrap().push(row);
        ids.push(id);
    }
    save_rows(&app, &state);
    let _ = handle.emit("downloads-changed", ());
    pump(&handle);
    ids
}

/// Whether two links are on the same host: a playlist's cookies go only
/// to its own site's videos (ADR-0005).
fn same_site(a: &str, b: &str) -> bool {
    crate::view::host_of(a).is_some_and(|host| crate::view::host_of(b) == Some(host))
}

/// A playlist's video as a waiting row: at its best, named after its
/// title as a file can be.
pub(super) fn new_row(id: u64, item: NewVideo, dest: &str) -> Row {
    let name = item
        .title
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .map_or_else(|| "video".into(), zajil_core::sanitize::sanitize_file_name);
    Row {
        id,
        url: item.url,
        name,
        folder: dest.to_owned(),
        choice: ChoiceView::Best,
        status: StatusView::Waiting,
        size: None,
        done: 0,
        speed: 0,
        error: None,
        file: None,
        private: false,
        move_to: None,
        label: None,
        added: Some(zajil_core::state::now_rfc3339()),
        queue: None,
        agents: Vec::new(),
    }
}

/// Runs video download `id` of the list; `ask` asks its page first (a
/// resumed one: the formats of the last probe may be gone, or stale).
pub(super) fn start<R: tauri::Runtime>(handle: tauri::AppHandle<R>, id: u64, ask: bool) {
    let state = handle.state::<VideoState>();
    let Some(mut row) = with_row(&state, id, |row| {
        row.status = StatusView::Active;
        row.clone()
    }) else {
        return;
    };
    let stop = Stop::new();
    state.downloads.lock().unwrap().insert(id, stop.clone());
    let known = state.formats.lock().unwrap().get(&row.url).cloned();
    // What was read ahead (the browser's button showed over it), if it fits.
    let ahead = {
        let cookies = state.cookies.lock().unwrap().contains_key(&row.url);
        let media = state.media.lock().unwrap().get(&row.url).cloned();
        state
            .ahead
            .lock()
            .unwrap()
            .find(&row.url, cookies, Instant::now())
            .map(|read| (read, media))
    };
    tauri::async_runtime::spawn(async move {
        let runner = runner(&handle.state::<crate::App>()).await;
        let events = handle.clone();
        let mut on_event = move |event: VideoEvent| match event {
            VideoEvent::Progress { done, total, speed } => {
                progressed(&events, id, done, total, speed);
            }
            // A part saved before was of another file: said in the list.
            VideoEvent::Note(note) => eprintln!("zajil: video {id}: {note}"),
            VideoEvent::StartedOver(why) => {
                eprintln!("zajil: video {id}: {why}");
                with_row(&events.state::<VideoState>(), id, |row| {
                    row.error = Some(why)
                });
            }
        };
        // Stopped before, Zajil's own way: continued from its parts with
        // the links saved there, without asking the page again (no yt-dlp,
        // which YouTube may stop without the browser's cookies).
        let saved_ids = match &row.choice {
            ChoiceView::Format { id } => vec![id.clone()],
            ChoiceView::Merge { video, audio } => vec![video.clone(), audio.clone()],
            _ => Vec::new(),
        };
        let resumed = if ask && !row.name.is_empty() && !saved_ids.is_empty() {
            let saved = zajil_core::video::Saved {
                page: &row.url,
                ids: &saved_ids,
                dest: std::path::Path::new(&row.folder),
                name: &row.name,
                agents: &row.agents,
            };
            runner.resume_saved(&saved, &stop, &mut on_event).await
        } else {
            None
        };
        let result = match resumed {
            Some(done) => done.map_err(VideoErrorView::from),
            None => {
                async {
                    // How long the page took before the first byte, for the
                    // log (owner's report, 2026-10-30: a few seconds).
                    let asked = Instant::now();
                    let had_ahead = ahead.is_some();
                    let read = match ahead {
                        Some((mut read, media)) => tokio::select! {
                            read = read.wait_for(Option::is_some) => read
                                .ok()
                                .and_then(|r| r.clone().flatten())
                                .filter(|sent| ahead_fits(sent, &row.url, media.as_deref())),
                            () = stop.requested() => None,
                        },
                        None => None,
                    };
                    let read_used = read.is_some();
                    let probed = match read {
                        Some(sent) => Ok(sent.probed),
                        None if known.is_some() && !ask => Err(None),
                        None => probe(
                            handle.state::<VideoState>().inner(),
                            &runner,
                            &row.url,
                            &stop,
                        )
                        .await
                        .map_err(Some),
                    };
                    eprintln!(
                        "zajil: video {id}: its page {} in {} ms",
                        match (read_used, had_ahead) {
                            (true, _) => "was read ahead",
                            (false, true) => "read ahead did not fit, read again",
                            (false, false) => "was read now",
                        },
                        asked.elapsed().as_millis()
                    );
                    let formats = match probed {
                        Err(None) => Ok(known.unwrap_or_default()),
                        Err(Some(e)) => Err(e.into()),
                        Ok(Probed::Video(video)) => {
                            // From the browser's list: named after its title, and
                            // its choice in yt-dlp's names.
                            if row.name.is_empty() {
                                let name = Some(video.title.trim())
                                    .filter(|t| !t.is_empty())
                                    .map_or_else(
                                        || "video".into(),
                                        zajil_core::sanitize::sanitize_file_name,
                                    );
                                let choice = from_list(&row.choice, &video.formats);
                                with_row(&handle.state::<VideoState>(), id, |r| {
                                    r.name = name.clone();
                                    r.choice = choice.clone();
                                });
                                row.name = name;
                                row.choice = choice;
                            }
                            Ok(video.formats)
                        }
                        Ok(Probed::Playlist { .. }) => Err(VideoErrorView {
                            kind: "playlist".into(),
                            message: "the page is a playlist".into(),
                        }),
                    };
                    // The engine's best, once chosen, is the row's choice from now on.
                    let chosen = formats.and_then(|formats| {
                        let wanted = still_offered(&row.choice, &formats);
                        if wanted != row.choice {
                            eprintln!("zajil: video {id}: its format is gone, the best goes");
                            row.choice = wanted;
                        }
                        let choice = row.choice.resolved(&formats)?;
                        if row.choice == ChoiceView::Best {
                            with_row(&handle.state::<VideoState>(), id, |row| {
                                row.choice = ChoiceView::from(choice.clone());
                            });
                        }
                        Ok((formats, choice))
                    });
                    // After the probe: the cookies it went with, if any.
                    let cookies = handle
                        .state::<VideoState>()
                        .cookies
                        .lock()
                        .unwrap()
                        .get(&row.url)
                        .cloned();
                    match chosen {
                        Ok((formats, choice)) => {
                            let agents: Vec<(String, String)> = formats
                                .iter()
                                .filter(|f| choice.ids().contains(&f.id.as_str()))
                                .filter_map(|f| Some((f.id.clone(), f.user_agent()?.to_owned())))
                                .collect();
                            with_row(&handle.state::<VideoState>(), id, |row| {
                                row.agents = agents;
                            });
                            let request = Download {
                                url: &row.url,
                                choice: &choice,
                                formats: &formats,
                                cookies: cookies.as_deref(),
                                dest: std::path::Path::new(&row.folder),
                                name: &row.name,
                            };
                            runner
                                .download_keeping(&request, Some(&key_of(id)), &stop, &mut on_event)
                                .await
                                .map_err(VideoErrorView::from)
                        }
                        Err(e) => Err(e),
                    }
                }
                .await
            }
        };
        // Finished either way: what yt-dlp kept for it is not needed.
        if result.is_ok() {
            let kept =
                zajil_core::video::kept_folder(std::path::Path::new(&row.folder), &key_of(id));
            let _ =
                tauri::async_runtime::spawn_blocking(move || zajil_core::video::remove_kept(&kept))
                    .await;
        }
        let state = handle.state::<VideoState>();
        state.downloads.lock().unwrap().remove(&id);
        state.told.lock().unwrap().remove(&id);
        // Cancelled while it ran: gone, with what it wrote.
        if state.discard.lock().unwrap().remove(&id) {
            let mut gone = row.clone();
            gone.file = result.as_ref().ok().map(|f| f.display().to_string());
            state.rows.lock().unwrap().retain(|r| r.id != id);
            save_rows(&handle.state::<crate::App>(), &state);
            let _ = handle.emit("downloads-changed", ());
            pump(&handle);
            let _ = tauri::async_runtime::spawn_blocking(move || remove_leftovers(&gone)).await;
            return;
        }
        // Finished where it downloaded: moved where the user said meanwhile.
        let move_to = with_row(&state, id, |row| {
            // Stopped: kept for when it finishes.
            if result.is_err() {
                return None;
            }
            row.move_to
                .take()
                .filter(|to| !row.lands_at(to))
                .map(|mut to| {
                    if to.name.is_empty() {
                        to.name = row.name.clone();
                    }
                    to
                })
        })
        .flatten();
        let result = match (result, move_to) {
            (Ok(file), Some(to)) => {
                let target = to.clone();
                let moved = tauri::async_runtime::spawn_blocking(move || {
                    zajil_core::video::move_video(
                        &file,
                        std::path::Path::new(&target.folder),
                        &target.name,
                    )
                    .map_err(|e| (e, file))
                })
                .await;
                match moved {
                    Ok(Ok(file)) => {
                        with_row(&state, id, |row| row.folder = to.folder.clone());
                        Ok(file)
                    }
                    // Left where it is, and said.
                    Ok(Err((e, file))) => {
                        eprintln!("zajil: video {id} could not be moved: {e}");
                        Ok(file)
                    }
                    Err(e) => Err(VideoErrorView {
                        kind: "io".into(),
                        message: e.to_string(),
                    }),
                }
            }
            (result, _) => result,
        };
        let lang = handle
            .state::<crate::App>()
            .settings
            .load()
            .map(|s| s.language)
            .unwrap_or_default();
        let requeued = state.requeue.lock().unwrap().remove(&id);
        with_row(&state, id, |row| match &result {
            Ok(file) => {
                row.status = StatusView::Done;
                row.error = None;
                row.file = Some(file.display().to_string());
                row.size = std::fs::metadata(file).ok().map(|m| m.len()).or(row.size);
                row.done = row.size.unwrap_or(row.done);
            }
            // Stopped by its queue's window: it waits for the next.
            Err(e) if e.kind == "stopped" && requeued => {
                row.status = StatusView::Waiting;
            }
            Err(e) if e.kind == "stopped" => {
                row.status = StatusView::Paused;
            }
            Err(e) => {
                // Its kind only: yt-dlp's words may hold the page's link.
                eprintln!("zajil: video {id} failed ({})", e.kind);
                row.status = StatusView::Failed;
                row.error = Some(e.text(lang));
            }
        });
        save_rows(&handle.state::<crate::App>(), &state);
        let _ = handle.emit("downloads-changed", ());
        // Its place is free: the next waiting one starts.
        pump(&handle);
        // The last of its queue: what the user chose for after the queue.
        if result.is_ok()
            && let Some(queue) = with_row(&state, id, |row| row.queue.clone()).flatten()
        {
            // To `schedule::video_done`, through lib.rs (it needs the app's
            // own runtime).
            let _ = handle.emit(QUEUE_VIDEO_DONE, queue);
        }
        match result {
            Ok(file) => {
                let _ = handle.emit(
                    "video-finished",
                    serde_json::json!({ "id": id, "file": file.display().to_string() }),
                );
            }
            Err(error) => {
                let _ = handle.emit(
                    "video-failed",
                    serde_json::json!({ "id": id, "error": error }),
                );
            }
        }
    });
}

/// A running video's progress: to «Video quality» and to the list, at most
/// twice a second (as the queues' downloads, `live.rs`), and its end at
/// once.
fn progressed<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    id: u64,
    done: u64,
    total: Option<u64>,
    speed: Option<u64>,
) {
    let state = handle.state::<VideoState>();
    with_row(&state, id, |row| {
        row.done = done;
        row.size = total;
        row.speed = speed.unwrap_or(0);
    });
    let now = Instant::now();
    let mut told = state.told.lock().unwrap();
    let end = total.is_some_and(|total| done >= total);
    if end
        || told
            .get(&id)
            .is_none_or(|at| now.duration_since(*at) >= crate::live::NOTIFY_EVERY)
    {
        told.insert(id, now);
        drop(told);
        let _ = handle.emit(
            "video-progress",
            Progress {
                id,
                done,
                total,
                speed,
            },
        );
        crate::downloads::tell_progress(handle);
    }
}

/// Stops a video download (yt-dlp and what it started end).
#[tauri::command]
pub fn video_cancel(state: State<'_, VideoState>, id: u64) {
    if let Some(stop) = state.downloads.lock().unwrap().remove(&id) {
        stop.request();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::tests::item;
    use zajil_core::video::{Choice, Format};

    /// Each one goes at the engine's best, named after its title as a
    /// file can be, and keeps that choice once its page is asked.
    #[test]
    fn a_playlist_s_video_goes_at_its_best_under_its_title() {
        let row = new_row(
            FIRST_ID + 1,
            item("https://example.com/v", Some("a/b: c")),
            "/d",
        );
        assert_eq!(row.choice, ChoiceView::Best);
        assert_eq!(row.status, StatusView::Waiting);
        assert!(
            !row.name.contains('/') && row.name.contains('c'),
            "{}",
            row.name
        );
        assert_eq!(
            new_row(FIRST_ID + 2, item("https://e.com/w", None), "/d").name,
            "video"
        );

        let format = |id: &str, height: u32, vcodec: &str, acodec: &str| {
            serde_json::from_value::<Format>(serde_json::json!({
                "format_id": id, "height": height, "vcodec": vcodec, "acodec": acodec
            }))
            .unwrap()
        };
        let formats = [
            format("18", 360, "avc1", "mp4a"),
            format("137", 1080, "avc1", "none"),
            format("140", 0, "none", "mp4a"),
        ];
        assert_eq!(
            ChoiceView::Best.resolved(&formats),
            Ok(Choice::Merge {
                video: "137".into(),
                audio: "140".into()
            })
        );
        assert_eq!(
            ChoiceView::Best.resolved(&[]).unwrap_err().kind,
            "no_formats"
        );
    }

    /// A playlist's cookies go to its own site's videos only.
    #[test]
    fn a_playlist_s_cookies_stay_on_its_site() {
        let page = "https://www.youtube.com/playlist?list=PLx";
        assert!(same_site(page, "https://www.youtube.com/watch?v=abc"));
        assert!(!same_site(page, "https://youtu.be.example.com/watch?v=abc"));
        assert!(!same_site(page, "not a link"));
        assert!(!same_site("not a link", "not a link"));
    }
}
