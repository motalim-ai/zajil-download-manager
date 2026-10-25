//! The schedule window (M5.5): the queues on the side, with «New queue»
//! and «Delete» under them; for the chosen queue, when it starts and
//! stops, how many of its downloads run at once, what happens after it
//! finishes, «Start now», and its downloads in order. A window of its own,
//! as the add windows (`windows.rs`), drawn by the same page
//! (`index.html?win=schedule`).
//!
//! On the M3 engine only: the queues file (`QueuesFile`) and its daily
//! `Schedule`; after the queue, `after_queue` with the system's
//! `PowerControl`, under the owner's M3 rules: only when the user asked,
//! for that queue (the choice is kept in memory, never saved); only if
//! every download of the queue is done; after a countdown the user can
//! cancel, shown in this window.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};
use ts_rs::TS;
use zajil_core::Stop;
use zajil_core::limit::SystemClock;
use zajil_core::queue::{
    AfterQueue, DEFAULT_QUEUE, Day, MAX_ACTIVE, QueueEvent, QueuesFile, Schedule, Status,
    StoreError, Summary, TimeOfDay, after_queue, check_name,
};
use zajil_platform::power::{PowerAction, SystemPower};

use crate::App;

/// The schedule window's capability: listening to Zajil's events, nothing
/// else. Added when the app starts, for this window only.
pub const CAPABILITY: &str = include_str!("../runtime-capabilities/schedule.json");

pub const LABEL: &str = "schedule";

/// The window's size, in logical pixels.
pub const SIZE: (f64, f64) = (900.0, 700.0);

/// How long the user has to cancel a shutdown or sleep after the queue.
const COUNTDOWN: Duration = Duration::from_secs(60);

/// What happens after a queue finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum AfterChoice {
    Nothing,
    Shutdown,
    Sleep,
}

impl AfterChoice {
    fn action(self) -> Option<PowerAction> {
        match self {
            AfterChoice::Nothing => None,
            AfterChoice::Shutdown => Some(PowerAction::Shutdown),
            AfterChoice::Sleep => Some(PowerAction::Sleep),
        }
    }

    fn of(action: PowerAction) -> Self {
        match action {
            PowerAction::Shutdown => AfterChoice::Shutdown,
            PowerAction::Sleep => AfterChoice::Sleep,
        }
    }
}

/// One queue as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct QueuePlan {
    pub name: String,
    pub max_active: u32,
    /// `HH:MM`; `null`: no start time (with no stop and no days either,
    /// the queue has no schedule and runs whenever it has downloads).
    pub start: Option<String>,
    pub stop: Option<String>,
    /// `mon` to `sun`; empty: every day.
    pub days: Vec<String>,
    /// It has a schedule and is outside its window now: «Start now» makes
    /// its downloads wait for the next window.
    pub closed_now: bool,
    /// What the user chose for after it, this run (never saved).
    pub after: AfterChoice,
}

/// What the window saves for a queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlanChange {
    pub max_active: u32,
    pub start: Option<String>,
    pub stop: Option<String>,
    pub days: Vec<String>,
}

/// The countdown after a queue, as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Countdown {
    pub queue: String,
    pub action: AfterChoice,
    /// Whole seconds left.
    pub seconds: u32,
}

/// What the user chose for after each queue, and the countdown running.
/// In memory only: nothing here survives a restart (owner's M3 rule).
#[derive(Debug, Default)]
pub struct After {
    chosen: Mutex<HashMap<String, PowerAction>>,
    /// The countdown, its number (to tell an older one), and its stop.
    running: Mutex<Option<(Countdown, u64, Stop)>>,
    counted: std::sync::atomic::AtomicU64,
    /// The window's title in the user's language, for opening it when a
    /// countdown starts.
    title: Mutex<String>,
}

pub type SharedAfter = Arc<After>;

/// Opens the schedule window titled `title`, or brings it to the front.
#[tauri::command]
pub fn open_schedule<R: tauri::Runtime>(
    after: State<'_, SharedAfter>,
    title: String,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    if crate::in_scene(&handle) {
        return Ok(());
    }
    *after.title.lock().unwrap() = title.clone();
    crate::windows::open(&handle, LABEL, &title, SIZE).map_err(|e| e.to_string())
}

/// The queues, with their schedules and what happens after each.
#[tauri::command]
pub async fn queue_plans(
    app: State<'_, App>,
    after: State<'_, SharedAfter>,
) -> Result<Vec<QueuePlan>, String> {
    let store = app.store.clone();
    let queues = tokio::task::spawn_blocking(move || store.load())
        .await
        .map_err(crate::blocking_err)?
        .map_err(|e| e.to_string())?;
    let chosen = after.chosen.lock().unwrap().clone();
    Ok(plans(
        &queues,
        &chosen,
        jiff::Timestamp::now(),
        &jiff::tz::TimeZone::system(),
    ))
}

fn plans(
    queues: &QueuesFile,
    chosen: &HashMap<String, PowerAction>,
    now: jiff::Timestamp,
    tz: &jiff::tz::TimeZone,
) -> Vec<QueuePlan> {
    queues
        .queues
        .iter()
        .map(|q| {
            let schedule = q.schedule.as_ref();
            QueuePlan {
                name: q.name.clone(),
                max_active: q.max_active,
                start: schedule.and_then(|s| s.start).map(short),
                stop: schedule.and_then(|s| s.stop).map(short),
                days: schedule
                    .map(|s| s.days.iter().map(Day::to_string).collect())
                    .unwrap_or_default(),
                closed_now: schedule.is_some_and(|s| !s.is_open(now, tz)),
                after: chosen
                    .get(&q.name)
                    .map_or(AfterChoice::Nothing, |a| AfterChoice::of(*a)),
            }
        })
        .collect()
}

/// `HH:MM`: the window's time boxes have no seconds.
fn short(time: TimeOfDay) -> String {
    let text = time.to_string();
    text.get(..5).unwrap_or(&text).to_owned()
}

/// «New queue» `name`: an existing name changes nothing.
#[tauri::command]
pub async fn new_queue<R: tauri::Runtime>(
    app: State<'_, App>,
    name: String,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    crate::change(&app, &handle, move |q| create(q, name.trim())).await
}

fn create(queues: &mut QueuesFile, name: &str) -> Result<(), StoreError> {
    check_name(name)?;
    if queues.queue(name).is_some() {
        return Err(StoreError::Refused(format!(
            "there is already a queue {name:?}"
        )));
    }
    queues.queue_or_new(name).map(|_| ())
}

/// «Delete» queue `name`: its downloads go to the end of the main queue,
/// in their order, so nothing is lost. The main queue stays.
#[tauri::command]
pub async fn delete_queue<R: tauri::Runtime>(
    app: State<'_, App>,
    after: State<'_, SharedAfter>,
    name: String,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    let gone = name.clone();
    crate::change(&app, &handle, move |q| delete(q, &name)).await?;
    after.chosen.lock().unwrap().remove(&gone);
    Ok(())
}

fn delete(queues: &mut QueuesFile, name: &str) -> Result<(), StoreError> {
    if name == DEFAULT_QUEUE {
        return Err(StoreError::Refused(
            "the main queue cannot be deleted".into(),
        ));
    }
    let at = queues
        .queues
        .iter()
        .position(|q| q.name == name)
        .ok_or_else(|| StoreError::NoSuchQueue(name.to_owned()))?;
    let gone = queues.queues.remove(at);
    queues.queue_or_new(DEFAULT_QUEUE)?.items.extend(gone.items);
    Ok(())
}

/// Saves queue `name`'s schedule and how many of its downloads run at
/// once. No start, no stop and no days: no schedule.
#[tauri::command]
pub async fn save_queue<R: tauri::Runtime>(
    app: State<'_, App>,
    name: String,
    change: PlanChange,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    let schedule = schedule_of(&change)?;
    crate::change(&app, &handle, move |q| {
        if !(1..=MAX_ACTIVE).contains(&change.max_active) {
            return Err(StoreError::InvalidMaxActive(change.max_active));
        }
        let queue = q
            .queue_mut(&name)
            .ok_or_else(|| StoreError::NoSuchQueue(name.clone()))?;
        queue.schedule = schedule;
        queue.max_active = change.max_active;
        Ok(())
    })
    .await
}

fn schedule_of(change: &PlanChange) -> Result<Option<Schedule>, String> {
    let time = |t: &Option<String>| -> Result<Option<TimeOfDay>, String> {
        t.as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| {
                t.parse()
                    .map_err(|e: zajil_core::queue::ScheduleError| e.to_string())
            })
            .transpose()
    };
    let (start, stop) = (time(&change.start)?, time(&change.stop)?);
    let days = change
        .days
        .iter()
        .map(|d| d.parse::<Day>().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    if start.is_none() && stop.is_none() && days.is_empty() {
        return Ok(None);
    }
    Schedule::new(start, stop, days)
        .map(Some)
        .map_err(|e| e.to_string())
}

/// Moves download `id` one place up (earlier) or down in its queue: the
/// order its queue starts them in.
#[tauri::command]
pub async fn move_in_queue<R: tauri::Runtime>(
    app: State<'_, App>,
    id: u64,
    up: bool,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    crate::change(&app, &handle, move |q| shift(q, id, up)).await
}

fn shift(queues: &mut QueuesFile, id: u64, up: bool) -> Result<(), StoreError> {
    for queue in &mut queues.queues {
        if let Some(at) = queue.items.iter().position(|i| i.id == id) {
            let to = if up { at.checked_sub(1) } else { Some(at + 1) };
            if let Some(to) = to.filter(|&to| to < queue.items.len()) {
                queue.items.swap(at, to);
            }
            return Ok(());
        }
    }
    Err(StoreError::NoSuchItem(id))
}

/// «Start now»: every paused or failed download of queue `name` waits to
/// start, as «Resume» does. Not one held for the user's OK (#84): that one
/// starts only when it is chosen. A queue outside its window starts them
/// at its next window (the engine has no «now» outside it).
#[tauri::command]
pub async fn start_queue<R: tauri::Runtime>(
    app: State<'_, App>,
    name: String,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    let store = app.store.clone();
    let ids = tokio::task::spawn_blocking(move || store.load().and_then(|q| startable(&q, &name)))
        .await
        .map_err(crate::blocking_err)?
        .map_err(|e| e.to_string())?;
    // A download whose link is being refreshed waits for the refresh, as
    // with «Resume».
    let ids = app.refreshing.without(ids);
    crate::change(&app, &handle, move |q| {
        crate::set_status(q, &ids, Status::Waiting)
    })
    .await
}

fn startable(queues: &QueuesFile, name: &str) -> Result<Vec<u64>, StoreError> {
    let queue = queues
        .queue(name)
        .ok_or_else(|| StoreError::NoSuchQueue(name.to_owned()))?;
    Ok(queue
        .items
        .iter()
        .filter(|i| !i.needs_ok && matches!(i.status, Status::Paused | Status::Failed))
        .map(|i| i.id)
        .collect())
}

/// What happens after queue `queue` finishes, this run only.
#[tauri::command]
pub fn set_after(after: State<'_, SharedAfter>, queue: String, choice: AfterChoice) {
    let mut chosen = after.chosen.lock().unwrap();
    match choice.action() {
        Some(action) => chosen.insert(queue, action),
        None => chosen.remove(&queue),
    };
}

/// The countdown running now, if any.
#[tauri::command]
pub fn countdown(after: State<'_, SharedAfter>) -> Option<Countdown> {
    after
        .running
        .lock()
        .unwrap()
        .as_ref()
        .map(|(c, _, _)| c.clone())
}

/// Cancels the countdown: the computer stays on.
#[tauri::command]
pub fn cancel_after(after: State<'_, SharedAfter>) {
    if let Some((_, _, stop)) = after.running.lock().unwrap().as_ref() {
        stop.request();
    }
}

/// The queue whose chosen action is due once download `item` finished:
/// the queue `item` is in, if the user chose an action for it and every
/// one of its downloads is done. The choice is taken: it happens once.
fn due(
    queues: &QueuesFile,
    item: u64,
    chosen: &mut HashMap<String, PowerAction>,
) -> Option<(String, PowerAction)> {
    let queue = queues
        .queues
        .iter()
        .find(|q| q.items.iter().any(|i| i.id == item))?;
    let all_done = queue.items.iter().all(|i| i.status == Status::Done);
    if queue.items.is_empty() || !all_done {
        return None;
    }
    let action = chosen.remove(&queue.name)?;
    Some((queue.name.clone(), action))
}

/// What the queue runner's events mean here: a download finished, so its
/// queue may be done, and the action the user chose for it is due.
pub fn on_event(handle: &tauri::AppHandle, event: &QueueEvent) {
    let QueueEvent::Finished { item, .. } = event else {
        return;
    };
    let (handle, item) = (handle.clone(), *item);
    tauri::async_runtime::spawn(async move {
        let after = handle.state::<SharedAfter>().inner().clone();
        if after.chosen.lock().unwrap().is_empty() {
            return;
        }
        let store = handle.state::<App>().store.clone();
        let Ok(Ok(queues)) = tokio::task::spawn_blocking(move || store.load()).await else {
            return;
        };
        let Some((queue, action)) = due(&queues, item, &mut after.chosen.lock().unwrap()) else {
            return;
        };
        count_down(&handle, &after, queue, action).await;
    });
}

/// The countdown in the schedule window, then the action, unless the user
/// cancels (or the queue turns out not finished: `after_queue` checks).
async fn count_down(handle: &tauri::AppHandle, after: &After, queue: String, action: PowerAction) {
    let stop = Stop::new();
    let number = after
        .counted
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let shown = Countdown {
        queue: queue.clone(),
        action: AfterChoice::of(action),
        seconds: COUNTDOWN.as_secs() as u32,
    };
    {
        let mut running = after.running.lock().unwrap();
        if let Some((_, _, old)) = running.replace((shown.clone(), number, stop.clone())) {
            // One countdown at a time: the newer one.
            old.request();
        }
    }
    let title = after.title.lock().unwrap().clone();
    let _ = crate::windows::open_beside_main(handle, LABEL, LABEL, &title, SIZE);
    let _ = handle.emit_to(LABEL, "after-countdown", Some(shown.clone()));
    let store = handle.state::<App>().store.clone();
    // The run ended with this queue: every download in it is done, the
    // last one just now. `after_queue` reads the file again to be sure.
    let summary = Summary {
        finished: 1,
        queue_ended: true,
        ..Summary::default()
    };
    let emitter = handle.clone();
    let outcome = after_queue(
        &store,
        &queue,
        &summary,
        &SystemPower,
        action,
        COUNTDOWN,
        &SystemClock::default(),
        &stop,
        &mut |left| {
            let seconds = (left.as_secs() + u64::from(left.subsec_nanos() > 0)) as u32;
            let now = Countdown {
                seconds,
                ..shown.clone()
            };
            let _ = emitter.emit_to(LABEL, "after-countdown", Some(now));
        },
    )
    .await;
    {
        let mut running = after.running.lock().unwrap();
        if running.as_ref().is_some_and(|(_, n, _)| *n == number) {
            *running = None;
        }
    }
    let _ = handle.emit_to(LABEL, "after-countdown", None::<Countdown>);
    let problem = match outcome {
        Ok(AfterQueue::Failed(err)) => Some(err.to_string()),
        Err(err) => Some(err.to_string()),
        Ok(_) => None,
    };
    if let Some(problem) = problem {
        eprintln!("zajil: after queue {queue}: {problem}");
        let _ = handle.emit_to(LABEL, "after-failed", problem);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn queues() -> QueuesFile {
        let mut q = QueuesFile::empty();
        let dir = Path::new(if cfg!(windows) { r"C:\d" } else { "/d" });
        for url in ["https://a.test/1", "https://a.test/2", "https://a.test/3"] {
            q.add(DEFAULT_QUEUE, url, dir).unwrap();
        }
        q.add("night", "https://a.test/4", dir).unwrap();
        q
    }

    fn ids(q: &QueuesFile, name: &str) -> Vec<u64> {
        q.queue(name).unwrap().items.iter().map(|i| i.id).collect()
    }

    #[test]
    fn a_new_queue_is_made_once_with_a_good_name() {
        let mut q = queues();
        create(&mut q, "evening").unwrap();
        assert!(q.queue("evening").unwrap().items.is_empty());
        assert!(create(&mut q, "evening").is_err());
        assert!(create(&mut q, "two words").is_err());
        assert!(create(&mut q, "").is_err());
        assert_eq!(q.queues.len(), 3);
    }

    #[test]
    fn deleting_a_queue_keeps_its_downloads_in_the_main_one() {
        let mut q = queues();
        let before = ids(&q, DEFAULT_QUEUE);
        let night = ids(&q, "night");
        delete(&mut q, "night").unwrap();
        assert!(q.queue("night").is_none());
        assert_eq!(ids(&q, DEFAULT_QUEUE), [before, night].concat());
        assert!(delete(&mut q, DEFAULT_QUEUE).is_err());
        assert!(delete(&mut q, "nowhere").is_err());
    }

    #[test]
    fn a_download_moves_up_and_down_within_its_queue() {
        let mut q = queues();
        let [a, b, c] = ids(&q, DEFAULT_QUEUE)[..] else {
            panic!()
        };
        shift(&mut q, c, true).unwrap();
        assert_eq!(ids(&q, DEFAULT_QUEUE), [a, c, b]);
        shift(&mut q, a, true).unwrap(); // already first
        shift(&mut q, b, false).unwrap(); // already last
        assert_eq!(ids(&q, DEFAULT_QUEUE), [a, c, b]);
        shift(&mut q, a, false).unwrap();
        assert_eq!(ids(&q, DEFAULT_QUEUE), [c, a, b]);
        assert!(shift(&mut q, 999, true).is_err());
    }

    #[test]
    fn the_window_saves_a_daily_schedule_or_none() {
        let change = |start: Option<&str>, stop: Option<&str>, days: &[&str]| PlanChange {
            max_active: 2,
            start: start.map(str::to_owned),
            stop: stop.map(str::to_owned),
            days: days.iter().map(|d| (*d).to_owned()).collect(),
        };
        assert_eq!(schedule_of(&change(None, Some(""), &[])), Ok(None));
        let s = schedule_of(&change(Some("02:00"), Some("07:30"), &["fri", "mon"]))
            .unwrap()
            .unwrap();
        assert_eq!(s.days, [Day::Mon, Day::Fri]);
        assert!(schedule_of(&change(Some("25:00"), None, &[])).is_err());
        assert!(schedule_of(&change(None, None, &["someday"])).is_err());

        let mut q = queues();
        q.queue_mut("night").unwrap().schedule = Some(s);
        let tz = jiff::tz::TimeZone::UTC;
        let at = |t: &str| t.parse::<jiff::Timestamp>().unwrap();
        // Monday 2026-10-26.
        let night = |now| plans(&q, &HashMap::new(), at(now), &tz).remove(1);
        let plan = night("2026-10-26T03:00:00Z");
        assert_eq!(plan.start.as_deref(), Some("02:00"));
        assert_eq!(plan.stop.as_deref(), Some("07:30"));
        assert_eq!(plan.days, ["mon", "fri"]);
        assert!(!plan.closed_now);
        assert!(night("2026-10-26T09:00:00Z").closed_now);
        assert!(!plans(&q, &HashMap::new(), at("2026-10-26T09:00:00Z"), &tz)[0].closed_now);
    }

    #[test]
    fn start_now_leaves_out_what_waits_for_the_users_ok() {
        let mut q = queues();
        let [a, b, c] = ids(&q, DEFAULT_QUEUE)[..] else {
            panic!()
        };
        q.item_mut(a).unwrap().status = Status::Paused;
        q.item_mut(b).unwrap().status = Status::Failed;
        q.item_mut(c).unwrap().status = Status::Paused;
        q.item_mut(c).unwrap().needs_ok = true;
        assert_eq!(startable(&q, DEFAULT_QUEUE).unwrap(), [a, b]);
        assert!(startable(&q, "nowhere").is_err());
    }

    /// The action is due only once every download of that queue is done,
    /// only for the queue the user chose it for, and only once.
    #[test]
    fn the_action_after_a_queue_waits_for_all_of_it_and_happens_once() {
        let mut q = queues();
        let main = ids(&q, DEFAULT_QUEUE);
        let night = ids(&q, "night")[0];
        let mut chosen = HashMap::from([("night".to_owned(), PowerAction::Sleep)]);

        q.item_mut(main[0]).unwrap().status = Status::Done;
        assert_eq!(due(&q, main[0], &mut chosen), None);
        for id in &main {
            q.item_mut(*id).unwrap().status = Status::Done;
        }
        // The main queue is done, but nothing was chosen for it.
        assert_eq!(due(&q, main[2], &mut chosen), None);
        assert_eq!(chosen.len(), 1);

        q.item_mut(night).unwrap().status = Status::Done;
        assert_eq!(
            due(&q, night, &mut chosen),
            Some(("night".to_owned(), PowerAction::Sleep))
        );
        assert_eq!(due(&q, night, &mut chosen), None);
    }

    /// The schedule window may listen to Zajil's events, and nothing more.
    #[test]
    fn the_schedule_window_has_only_its_own_permissions() {
        let capability: serde_json::Value = serde_json::from_str(CAPABILITY).unwrap();
        assert_eq!(capability["windows"], serde_json::json!([LABEL]));
        assert_eq!(
            capability["permissions"],
            serde_json::json!(["core:event:default"])
        );
    }
}
