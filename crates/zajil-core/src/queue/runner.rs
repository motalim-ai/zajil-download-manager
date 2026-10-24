//! Running the queues: each queue starts its waiting items in order, never
//! more than its `max_active` at once.
//!
//! The queues file is the only way to steer a runner, for every interface:
//! `zajil queue pause 3` (another process) changes the item's status, and
//! the runner sees it at its next look (every [`Runner::poll`]). A paused
//! or removed item is stopped with [`Stop`]: what is on disk is recorded
//! and nothing of it runs any more when its status is written ("stop and
//! wait", #5), so resuming it at once is safe.
//!
//! A runner killed in the middle leaves items `active` in the file; the
//! next runner puts them back to waiting, and they resume from their state
//! files.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::store::{QueueStore, QueuesFile, Status, StoreError};
use crate::category::CategoryStore;
use crate::limit::SpeedLimit;
use crate::{Engine, Error, Event, Finished, Stop};

/// What a runner reports. Interfaces show these; the runner never prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueEvent {
    /// Item `item` of `queue` starts (or resumes).
    Started {
        queue: String,
        item: u64,
    },
    /// An event of the download of `item`.
    Download {
        item: u64,
        event: Event,
    },
    Finished {
        item: u64,
        file: PathBuf,
    },
    /// The download stopped with `error`; the item is `failed`.
    Failed {
        item: u64,
        error: String,
    },
    /// The download was stopped (paused, removed, its queue's window
    /// closed, or the runner stops).
    Stopped {
        item: u64,
    },
    /// `queue` has downloads but is outside its window until `until`
    /// (local time).
    Waiting {
        queue: String,
        until: jiff::Zoned,
    },
}

/// What the schedules say at one look.
#[derive(Debug, Default)]
struct Windows {
    /// Queues outside their window.
    closed: HashSet<String>,
    /// When the next window opens or closes, on the engine's clock.
    next_change: Option<Duration>,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Summary {
    pub finished: usize,
    pub failed: usize,
    /// The run was asked to stop before the queues were done.
    pub stopped: bool,
    /// The run ended because the queue of [`Runner::until_queue_done`]
    /// finished: every download in it is done, at least one of them in
    /// this run (other downloads were stopped and wait).
    pub queue_ended: bool,
}

/// Runs every queue of one [`QueueStore`].
#[derive(Debug, Clone)]
pub struct Runner {
    engine: Engine,
    store: QueueStore,
    poll: Duration,
    /// The local time of schedules.
    time_zone: TimeZone,
    /// End the run when this queue has nothing left to run.
    until: Option<String>,
    /// Where items added without a folder of their own find their
    /// category's folder (M4).
    categories: Option<CategoryStore>,
    /// Connections for each download, read when it starts (M4).
    connections: Option<Arc<AtomicUsize>>,
}

/// How the download of an item ended.
type Ended = (u64, Result<Finished, Error>);

/// A download the runner started, and how to stop it.
struct Running {
    /// Its task, to know which item a task that panicked was.
    task: tokio::task::Id,
    stop: Stop,
    /// The runner itself asked it to stop: it goes back to waiting.
    by_runner: bool,
}

impl Runner {
    pub fn new(engine: Engine, store: QueueStore) -> Self {
        Runner {
            engine,
            store,
            poll: Duration::from_millis(500),
            time_zone: TimeZone::system(),
            until: None,
            categories: None,
            connections: None,
        }
    }

    /// Items marked `categorize` go to their category's folder, read from
    /// `store` when each of them starts.
    pub fn categories(mut self, store: CategoryStore) -> Self {
        self.categories = Some(store);
        self
    }

    /// Each download takes its number of connections from `count` when it
    /// starts, so a setting changed while the queues run applies to the
    /// next download (the desktop's settings, M4). Without it, the
    /// engine's own number.
    pub fn connections(mut self, count: Arc<AtomicUsize>) -> Self {
        self.connections = Some(count);
        self
    }

    /// Ends the run as soon as `queue` has nothing left to run, stopping
    /// (and waiting for) the downloads of other queues, which go back to
    /// waiting. For an action after that queue ([`super::after_queue`]).
    pub fn until_queue_done(mut self, queue: &str) -> Self {
        self.until = Some(queue.to_owned());
        self
    }

    /// The time zone of schedules (the system's by default).
    pub fn time_zone(mut self, time_zone: TimeZone) -> Self {
        self.time_zone = time_zone;
        self
    }

    /// How often the queues file is read for changes made by others.
    pub fn poll(mut self, every: Duration) -> Self {
        self.poll = every;
        self
    }

    /// Runs the queues until no item is waiting or active, or until `stop`
    /// is requested: then every download is stopped and waited for, and
    /// goes back to waiting for the next run. Fails at once with
    /// [`StoreError::RunnerBusy`] if another runner runs.
    pub async fn run(
        &self,
        stop: &Stop,
        on_event: &mut (dyn FnMut(QueueEvent) + Send),
    ) -> Result<Summary, StoreError> {
        let store = self.store.clone();
        let _runner = blocking(move || store.lock_runner()).await?;
        // Items left active by a runner that was killed.
        self.update(|queues| {
            for item in queues.queues.iter_mut().flat_map(|q| &mut q.items) {
                if item.status == Status::Active {
                    item.status = Status::Waiting;
                }
            }
            Ok(())
        })
        .await?;

        // The program-wide limit: one bucket for every download of this
        // run, its rate read from the settings at each look.
        let speed_limit = SpeedLimit::new(None, self.engine.clock());
        let engine = self.engine.with_speed_limit(Some(speed_limit.clone()));
        let (events, mut messages) = mpsc::unbounded_channel::<(u64, Event)>();
        let mut tasks: JoinSet<Ended> = JoinSet::new();
        let mut running: HashMap<u64, Running> = HashMap::new();
        let mut summary = Summary::default();
        let mut tick = tokio::time::interval(self.poll);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Whether to read the queues file again: at each tick and after
        // each change, but not for every progress event.
        let mut look = true;
        // When a queue's window next opens or closes (on the engine's
        // clock), and what was last said about each closed queue.
        let mut change_at: Option<Duration> = None;
        let mut announced: HashMap<String, Timestamp> = HashMap::new();
        // Items whose download finished in this run.
        let mut finished_now: HashSet<u64> = HashSet::new();
        loop {
            if look {
                look = false;
                let store = self.store.clone();
                let queues = blocking(move || store.load()).await?;
                speed_limit.set_rate(queues.settings.speed_limit);
                if let Some(until) = &self.until
                    && !summary.stopped
                    && !summary.queue_ended
                {
                    let queue = queues
                        .queue(until)
                        .ok_or_else(|| StoreError::NoSuchQueue(until.clone()))?;
                    // Finished: all done, and not all before this run (a
                    // queue finished earlier must not turn the computer
                    // off at once). A failed or paused download is not
                    // done, so the run then goes on as if nothing had been
                    // asked, and nothing else is stopped (review of #35).
                    let all_done = !queue.items.is_empty()
                        && queue
                            .items
                            .iter()
                            .all(|i| i.status == Status::Done && !running.contains_key(&i.id));
                    let one_now = queue.items.iter().any(|i| finished_now.contains(&i.id));
                    if all_done && one_now {
                        summary.queue_ended = true;
                        for run in running.values_mut() {
                            run.by_runner = true;
                            run.stop.request();
                        }
                    }
                }
                let windows = self.windows(&queues, &mut announced, on_event);
                change_at = windows.next_change;
                self.stop_unwanted(&queues, &windows.closed, &mut running);
                if !summary.stopped && !summary.queue_ended {
                    self.start_waiting(
                        &engine,
                        &queues,
                        &windows.closed,
                        &mut running,
                        &mut tasks,
                        &events,
                        on_event,
                    )
                    .await?;
                }
                if running.is_empty()
                    && (summary.stopped || summary.queue_ended || !has_waiting(&queues))
                {
                    return Ok(summary);
                }
            }
            let clock = self.engine.clock();
            let next_window = async {
                match change_at {
                    Some(at) => clock.sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                Some(joined) = tasks.join_next_with_id() => {
                    // Events the task sent before it ended come first: a
                    // late `Started` must not record a state file after the
                    // end was recorded (review of #31).
                    while let Ok((item, event)) = messages.try_recv() {
                        self.download_event(item, event, &running, on_event).await?;
                    }
                    self.finished(joined, &mut running, &mut summary, &mut finished_now, on_event)
                        .await?;
                    look = true;
                }
                Some((item, event)) = messages.recv() => {
                    self.download_event(item, event, &running, on_event).await?;
                }
                () = stop.requested(), if !summary.stopped => {
                    summary.stopped = true;
                    for run in running.values_mut() {
                        run.by_runner = true;
                        run.stop.request();
                    }
                    look = true;
                }
                _ = tick.tick() => look = true,
                () = next_window => look = true,
            }
        }
    }

    /// Which queues with work are outside their window now, and when the
    /// next window opens or closes (as a time of the engine's clock).
    /// Tells once about each queue that waits for its window.
    fn windows(
        &self,
        queues: &QueuesFile,
        announced: &mut HashMap<String, Timestamp>,
        on_event: &mut (dyn FnMut(QueueEvent) + Send),
    ) -> Windows {
        let clock = self.engine.clock();
        let (now, wall) = (clock.now(), clock.wall());
        let mut windows = Windows::default();
        for queue in &queues.queues {
            let Some(schedule) = &queue.schedule else {
                continue;
            };
            let has_work = queue
                .items
                .iter()
                .any(|i| matches!(i.status, Status::Waiting | Status::Active));
            if !has_work {
                announced.remove(&queue.name);
                continue;
            }
            let change = schedule.next_change(wall, &self.time_zone);
            let wait = Duration::try_from(change.duration_since(wall)).unwrap_or(Duration::ZERO);
            let at = now.saturating_add(wait);
            windows.next_change = Some(windows.next_change.map_or(at, |next| next.min(at)));
            if schedule.is_open(wall, &self.time_zone) {
                announced.remove(&queue.name);
            } else {
                windows.closed.insert(queue.name.clone());
                if announced.insert(queue.name.clone(), change) != Some(change) {
                    on_event(QueueEvent::Waiting {
                        queue: queue.name.clone(),
                        until: change.to_zoned(self.time_zone.clone()),
                    });
                }
            }
        }
        windows
    }

    /// Stops the downloads whose items the user paused or removed (or set
    /// to anything but active), and those of queues whose window closed:
    /// those go back to waiting.
    fn stop_unwanted(
        &self,
        queues: &QueuesFile,
        closed: &HashSet<String>,
        running: &mut HashMap<u64, Running>,
    ) {
        for queue in &queues.queues {
            for item in &queue.items {
                if let Some(run) = running.get_mut(&item.id)
                    && item.status == Status::Active
                    && closed.contains(&queue.name)
                {
                    run.by_runner = true;
                    run.stop.request();
                }
            }
        }
        for (id, run) in running.iter() {
            let wanted = queues
                .item(*id)
                .is_some_and(|item| item.status == Status::Active);
            if !wanted {
                run.stop.request();
            }
        }
    }

    /// Starts waiting items, in order, while their queue has room.
    #[allow(clippy::too_many_arguments)]
    async fn start_waiting(
        &self,
        engine: &Engine,
        queues: &QueuesFile,
        closed: &HashSet<String>,
        running: &mut HashMap<u64, Running>,
        tasks: &mut JoinSet<Ended>,
        events: &mpsc::UnboundedSender<(u64, Event)>,
        on_event: &mut (dyn FnMut(QueueEvent) + Send),
    ) -> Result<(), StoreError> {
        for queue in queues.queues.iter().filter(|q| !closed.contains(&q.name)) {
            let busy = queue
                .items
                .iter()
                .filter(|i| running.contains_key(&i.id))
                .count();
            let room = (queue.max_active as usize).saturating_sub(busy);
            let next: Vec<_> = queue
                .items
                .iter()
                .filter(|i| i.status == Status::Waiting && !running.contains_key(&i.id))
                .take(room)
                .map(|i| i.id)
                .collect();
            for id in next {
                // Marked active first: if the user changed it meanwhile,
                // it is not started.
                let item = self
                    .update(move |queues| {
                        let item = queues.item_mut(id).ok_or(StoreError::NoSuchItem(id))?;
                        if item.status != Status::Waiting {
                            return Ok(None);
                        }
                        item.status = Status::Active;
                        item.error = None;
                        Ok(Some(item.clone()))
                    })
                    .await;
                let item = match item {
                    Ok(Some(item)) => item,
                    Ok(None) | Err(StoreError::NoSuchItem(_)) => continue,
                    Err(err) => return Err(err),
                };
                on_event(QueueEvent::Started {
                    queue: queue.name.clone(),
                    item: id,
                });
                let stop = Stop::new();
                let mut engine = engine.with_download_limit(item.limit);
                if let Some(count) = &self.connections {
                    engine = engine.with_connections(count.load(Ordering::Relaxed));
                }
                let categories = self.categories.clone().filter(|_| item.categorize);
                let events = events.clone();
                let task_stop = stop.clone();
                let task = tasks.spawn(async move {
                    let stop = task_stop;
                    let mut report = |event| {
                        let _ = events.send((id, event));
                    };
                    let resumable = match item.state_file.clone() {
                        Some(path) => {
                            let url = item.url.clone();
                            tokio::task::spawn_blocking(move || belongs_to(&path, &url))
                                .await
                                .unwrap_or(false)
                        }
                        None => false,
                    };
                    let result = match item.state_file.filter(|_| resumable) {
                        Some(state_file) => {
                            engine.resume_until(&state_file, &stop, &mut report).await
                        }
                        None => match categories {
                            None => {
                                engine
                                    .download_until(&item.url, &item.dir, &stop, &mut report)
                                    .await
                            }
                            Some(store) => {
                                match tokio::task::spawn_blocking(move || store.load()).await {
                                    Ok(Ok(categories)) => {
                                        let given = item.dir.clone();
                                        let choose = move |name: &str, url: &reqwest::Url| {
                                            categories.folder_for(name, url.host_str(), &given)
                                        };
                                        engine
                                            .download_choosing_until(
                                                &item.url,
                                                &choose,
                                                &stop,
                                                &mut report,
                                            )
                                            .await
                                    }
                                    Ok(Err(err)) => {
                                        Err(Error::Io(std::io::Error::other(err.to_string())))
                                    }
                                    Err(err) => {
                                        Err(Error::Io(std::io::Error::other(err.to_string())))
                                    }
                                }
                            }
                        },
                    };
                    (id, result)
                });
                running.insert(
                    id,
                    Running {
                        task: task.id(),
                        stop,
                        by_runner: false,
                    },
                );
            }
        }
        Ok(())
    }

    /// Records where a download keeps its state, and passes its events on.
    async fn download_event(
        &self,
        id: u64,
        event: Event,
        running: &HashMap<u64, Running>,
        on_event: &mut (dyn FnMut(QueueEvent) + Send),
    ) -> Result<(), StoreError> {
        if !running.contains_key(&id) {
            return Ok(()); // a download that has ended
        }
        if let Event::Started { state_file, .. } = &event {
            let state_file = state_file.clone();
            ignore_removed(
                self.update(move |queues| {
                    let item = queues.item_mut(id).ok_or(StoreError::NoSuchItem(id))?;
                    item.state_file = Some(state_file);
                    Ok(())
                })
                .await,
            )?;
        }
        on_event(QueueEvent::Download { item: id, event });
        Ok(())
    }

    /// Records how a download ended.
    async fn finished(
        &self,
        joined: Result<(tokio::task::Id, Ended), tokio::task::JoinError>,
        running: &mut HashMap<u64, Running>,
        summary: &mut Summary,
        finished_now: &mut HashSet<u64>,
        on_event: &mut (dyn FnMut(QueueEvent) + Send),
    ) -> Result<(), StoreError> {
        let (id, result) = match joined {
            Ok((_, done)) => done,
            Err(err) => {
                // A download task panicked: find its item by its task.
                let Some(id) = running
                    .iter()
                    .find(|(_, run)| run.task == err.id())
                    .map(|(id, _)| *id)
                else {
                    return Ok(());
                };
                let error = io_error(format!("the download stopped unexpectedly: {err}"));
                (id, Err(Error::Io(error)))
            }
        };
        let by_runner = running.remove(&id).is_some_and(|run| run.by_runner);
        let event = match &result {
            Ok(done) => {
                summary.finished += 1;
                finished_now.insert(id);
                QueueEvent::Finished {
                    item: id,
                    file: done.file.clone(),
                }
            }
            Err(Error::Stopped { .. }) => QueueEvent::Stopped { item: id },
            Err(err) => {
                summary.failed += 1;
                QueueEvent::Failed {
                    item: id,
                    error: err.to_string(),
                }
            }
        };
        ignore_removed(
            self.update(move |queues| {
                let item = queues.item_mut(id).ok_or(StoreError::NoSuchItem(id))?;
                match result {
                    Ok(done) => {
                        item.status = Status::Done;
                        item.file = Some(done.file);
                        item.state_file = None;
                        item.error = None;
                    }
                    Err(Error::Stopped { state_file }) => {
                        // The user's own change (paused) stays; a stop by
                        // the runner puts it back to waiting.
                        if by_runner || item.status == Status::Active {
                            item.status = Status::Waiting;
                        }
                        // `None`: nothing was written and the files are
                        // gone; the name may soon be another download's.
                        item.state_file = state_file;
                    }
                    Err(err) => {
                        item.status = Status::Failed;
                        match &err {
                            Error::Interrupted { state_file, .. } => {
                                item.state_file = Some(state_file.clone());
                            }
                            // Nothing written, files removed (review of #31).
                            Error::NoData(_) => item.state_file = None,
                            _ => {}
                        }
                        item.error = Some(err.to_string());
                    }
                }
                Ok(())
            })
            .await,
        )?;
        on_event(event);
        Ok(())
    }

    async fn update<T: Send + 'static>(
        &self,
        change: impl FnOnce(&mut QueuesFile) -> Result<T, StoreError> + Send + 'static,
    ) -> Result<T, StoreError> {
        let store = self.store.clone();
        blocking(move || store.update(change)).await
    }
}

/// Whether the state file at `path` is this item's download: the file may
/// have been removed (a new download stopped before its first byte), and
/// its name taken by another download since, of another URL (review of
/// #31). A missing file or one of another URL is not; a file that cannot
/// be read is left to the resume, which says why.
fn belongs_to(path: &std::path::Path, url: &str) -> bool {
    match crate::state::DownloadState::load(path) {
        Ok(state) => reqwest::Url::parse(url).is_ok_and(|url| url.as_str() == state.url),
        Err(crate::state::StateError::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {
            false
        }
        Err(_) => true,
    }
}

fn io_error(message: String) -> std::io::Error {
    std::io::Error::other(message)
}

fn has_waiting(queues: &QueuesFile) -> bool {
    queues
        .queues
        .iter()
        .flat_map(|q| &q.items)
        .any(|i| matches!(i.status, Status::Waiting | Status::Active))
}

/// An item the user removed meanwhile is not an error.
fn ignore_removed(result: Result<(), StoreError>) -> Result<(), StoreError> {
    match result {
        Err(StoreError::NoSuchItem(_)) => Ok(()),
        other => other,
    }
}

/// Runs file work (the queues file and its lock) off the async threads.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, StoreError> + Send + 'static,
) -> Result<T, StoreError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| StoreError::Io(std::io::Error::other(err)))?
}
