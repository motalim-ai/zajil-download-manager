//! Video downloads in queues: pausing, resuming, and starting them when
//! their queue may run.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use tauri::{Emitter, Manager};

use super::download::start;
use super::rows::{save_rows, with_row};
use super::{Row, VideoState, at_once};
use crate::view::StatusView;

/// «Pause»: a running video stops (it continues when resumed); a waiting
/// one stays out of the turn.
pub fn pause(state: &VideoState, ids: &[u64]) {
    for &id in ids {
        with_row(state, id, |row| {
            if row.status == StatusView::Waiting {
                row.status = StatusView::Paused;
            }
        });
    }
    let downloads = state.downloads.lock().unwrap();
    for id in ids {
        if let Some(stop) = downloads.get(id) {
            stop.request();
        }
    }
}

/// «Resume»: a stopped or failed video starts again, from asking its page.
pub fn resume<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, ids: &[u64]) {
    let state = handle.state::<VideoState>();
    for &id in ids {
        with_row(&state, id, |row| {
            if matches!(row.status, StatusView::Paused | StatusView::Failed) {
                row.status = StatusView::Waiting;
                row.error = None;
            }
        });
    }
    save_rows(&handle.state::<crate::App>(), &state);
    let _ = handle.emit("downloads-changed", ());
    pump(handle);
}

/// What a queue allows its videos now: whether it may run (no schedule,
/// its window open, or started now), and how many of its downloads may
/// run besides those of its own list that run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Gate {
    open: bool,
    room: usize,
}

fn gates(
    queues: &zajil_core::queue::QueuesFile,
    run_now: &HashSet<String>,
    now: jiff::Timestamp,
    tz: &jiff::tz::TimeZone,
) -> HashMap<String, Gate> {
    queues
        .queues
        .iter()
        .map(|q| {
            let open =
                q.schedule.as_ref().is_none_or(|s| s.is_open(now, tz)) || run_now.contains(&q.name);
            let active = q
                .items
                .iter()
                .filter(|i| i.status == zajil_core::queue::Status::Active)
                .count();
            let room = (q.max_active as usize).saturating_sub(active);
            (q.name.clone(), Gate { open, room })
        })
        .collect()
}

/// The queues' gates now, from the queues file (none if it cannot be read:
/// a video in a queue then waits, rather than run outside its window).
fn gates_now<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) -> Option<HashMap<String, Gate>> {
    let queues = handle.state::<crate::App>().store.load().ok()?;
    let run_now = handle
        .state::<crate::schedule::SharedAfter>()
        .run_now
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    Some(gates(
        &queues,
        &run_now,
        jiff::Timestamp::now(),
        &jiff::tz::TimeZone::system(),
    ))
}

/// The waiting video downloads that may start now, first come first: as
/// many as leave at most `most` running, and a video in a queue only while
/// its queue may run and has room (a queue gone: as in none).
fn to_start(
    rows: &[Row],
    running: &HashSet<u64>,
    most: usize,
    gates: &HashMap<String, Gate>,
) -> Vec<u64> {
    let free = most.saturating_sub(running.len());
    let mut in_queue: HashMap<&str, usize> = HashMap::new();
    for row in rows.iter().filter(|r| running.contains(&r.id)) {
        if let Some(queue) = &row.queue {
            *in_queue.entry(queue).or_default() += 1;
        }
    }
    let mut ids = Vec::new();
    for row in rows
        .iter()
        .filter(|r| r.status == StatusView::Waiting && !running.contains(&r.id))
    {
        if ids.len() >= free {
            break;
        }
        if let Some(gate) = row
            .queue
            .as_deref()
            .and_then(|q| gates.get(q).map(|g| (q, g)))
        {
            let (queue, gate) = gate;
            let taken = in_queue.entry(queue).or_default();
            if !gate.open || *taken >= gate.room {
                continue;
            }
            *taken += 1;
        }
        ids.push(row.id);
    }
    ids
}

/// Every half minute: the videos of a queue whose window closed go back
/// to waiting, and those whose window opened start.
pub fn watch_queues<R: tauri::Runtime>(handle: tauri::AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let state = handle.state::<VideoState>();
            let in_queues = state.rows.lock().unwrap().iter().any(|r| r.queue.is_some());
            if !in_queues {
                continue;
            }
            if let Some(gates) = gates_now(&handle) {
                let closed: Vec<u64> = state
                    .rows
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|r| r.status == StatusView::Active)
                    .filter(|r| {
                        r.queue
                            .as_deref()
                            .and_then(|q| gates.get(q))
                            .is_some_and(|g| !g.open)
                    })
                    .map(|r| r.id)
                    .collect();
                let downloads = state.downloads.lock().unwrap();
                for id in closed {
                    if let Some(stop) = downloads.get(&id) {
                        state.requeue.lock().unwrap().insert(id);
                        stop.request();
                    }
                }
            }
            pump(&handle);
        }
    });
}

/// Whether every video of `queue` is done (none: done).
pub fn queue_videos_done(state: &VideoState, queue: &str) -> bool {
    state
        .rows
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.queue.as_deref() == Some(queue))
        .all(|r| r.status == StatusView::Done)
}

/// Starts the waiting video downloads whose turn it is.
pub(super) fn pump<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    let state = handle.state::<VideoState>();
    let running = state.downloads.lock().unwrap().keys().copied().collect();
    let in_queues = state.rows.lock().unwrap().iter().any(|r| r.queue.is_some());
    let gates = if in_queues {
        gates_now(handle)
    } else {
        Some(HashMap::new())
    };
    // The queues file unreadable: the videos of queues wait, the others go.
    let gates = gates.unwrap_or_else(|| {
        state
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter_map(|r| r.queue.clone())
            .map(|q| {
                (
                    q,
                    Gate {
                        open: false,
                        room: 0,
                    },
                )
            })
            .collect()
    });
    let ids = to_start(
        &state.rows.lock().unwrap(),
        &running,
        at_once(&state),
        &gates,
    );
    for id in ids {
        // A page «Video quality» asked just now is not asked again.
        let ask = !state.fresh.lock().unwrap().remove(&id);
        start(handle.clone(), id, ask);
    }
}

/// `queue` if the queues file has it, else none (the videos' own list).
pub(super) fn known_queue(app: &crate::App, queue: Option<String>) -> Option<String> {
    let queue = queue.filter(|q| !q.is_empty())?;
    let queues = app.store.load().ok()?;
    queues.queue(&queue).map(|_| queue)
}

/// Puts video downloads `ids` in `queue` (none: the videos' own list), as
/// «Add to queue» does for the others: they start when it may run.
pub fn set_queue<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    ids: &[u64],
    queue: Option<String>,
) {
    let state = handle.state::<VideoState>();
    let queue = known_queue(&handle.state::<crate::App>(), queue);
    for &id in ids {
        with_row(&state, id, |row| row.queue = queue.clone());
    }
    save_rows(&handle.state::<crate::App>(), &state);
    let _ = handle.emit("downloads-changed", ());
    pump(handle);
}

/// The event a finished video of a queue sends, for its queue's «after».
pub const QUEUE_VIDEO_DONE: &str = "video-queue-done";

/// [`set_queue`] for the page («Remove from queue»: none).
#[tauri::command]
pub fn video_set_queue<R: tauri::Runtime>(
    handle: tauri::AppHandle<R>,
    ids: Vec<u64>,
    queue: Option<String>,
) {
    set_queue(&handle, &ids, queue);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::FIRST_ID;
    use crate::video::download::new_row;
    use crate::video::tests::item;

    /// A playlist's chosen videos all join the list at once, and at most
    /// the settings' number run: the rest wait their turn, first come
    /// first.
    #[test]
    fn a_playlist_s_videos_wait_their_turn() {
        let mut rows: Vec<Row> = (1..=5)
            .map(|n| new_row(FIRST_ID + n, item("https://example.com/v", Some("v")), "/d"))
            .collect();
        let none = HashSet::new();
        let free = HashMap::new();
        assert_eq!(
            to_start(&rows, &none, 2, &free),
            [FIRST_ID + 1, FIRST_ID + 2]
        );
        assert_eq!(to_start(&rows, &none, 3, &free).len(), 3);
        let two: HashSet<u64> = [FIRST_ID + 1, FIRST_ID + 2].into();
        for row in &mut rows[..2] {
            row.status = StatusView::Active;
        }
        assert!(to_start(&rows, &two, 2, &free).is_empty());
        // One ended, one paused: the next waiting one, not the paused.
        rows[0].status = StatusView::Done;
        rows[2].status = StatusView::Paused;
        let one: HashSet<u64> = [FIRST_ID + 2].into();
        assert_eq!(to_start(&rows, &one, 2, &free), [FIRST_ID + 4]);
    }

    /// A video in a queue starts only while its queue may run and has
    /// room; one in no queue, or in a queue gone, as before.
    #[test]
    fn a_video_in_a_queue_follows_its_queue() {
        let mut rows: Vec<Row> = (1..=4)
            .map(|n| new_row(FIRST_ID + n, item("https://example.com/v", Some("v")), "/d"))
            .collect();
        rows[0].queue = Some("night".into());
        rows[1].queue = Some("night".into());
        rows[2].queue = Some("gone".into());
        let none = HashSet::new();
        let closed: HashMap<String, Gate> = [(
            "night".to_owned(),
            Gate {
                open: false,
                room: 3,
            },
        )]
        .into();
        assert_eq!(
            to_start(&rows, &none, 8, &closed),
            [FIRST_ID + 3, FIRST_ID + 4]
        );
        let one_room: HashMap<String, Gate> = [(
            "night".to_owned(),
            Gate {
                open: true,
                room: 1,
            },
        )]
        .into();
        assert_eq!(
            to_start(&rows, &none, 8, &one_room),
            [FIRST_ID + 1, FIRST_ID + 3, FIRST_ID + 4]
        );
        // One of the queue's own videos runs: no room left in it.
        let running: HashSet<u64> = [FIRST_ID + 1].into();
        rows[0].status = StatusView::Active;
        assert_eq!(
            to_start(&rows, &running, 8, &one_room),
            [FIRST_ID + 3, FIRST_ID + 4]
        );
    }
}
