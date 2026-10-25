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
    /// Every download of `queue` is done, one of them finished since the
    /// runner last looked (#142): said once, until the queue has work again.
    QueueDone {
        queue: String,
    },
    /// The download of `item` failed with `error`, and its queue gives it
    /// another try (#142): it waits at the end of its queue, after a pause.
    Retrying {
        item: u64,
        error: String,
    },
    /// `queue` has downloads but is outside its window until `until`
    /// (local time).
    Waiting {
        queue: String,
        until: jiff::Zoned,
    },
    /// Item `item` came from the browser with its cookies, and has none in
    /// memory (Zajil was restarted): it waits for that browser to give
    /// them again (M5). Said at every look while it waits.
    NeedsBrowser {
        item: u64,
        /// Its link, whose cookies are wanted.
        url: String,
        source: crate::queue::BrowserSource,
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
    /// The browser's headers of downloads the extension sent, by item
    /// (M5): in memory only, gone when the download finishes.
    from_browser: Option<FromBrowserById>,
    /// Queues started now, outside their window (#125).
    run_now: Option<RunNow>,
    /// Items that failed and wait for another try (#142): not started
    /// before this time of the engine's clock.
    retry_at: Arc<std::sync::Mutex<HashMap<u64, Duration>>>,
}

/// Queues the user started now (#125, «Start now»): each runs as if its
/// window were open until it has nothing waiting or running; its saved
/// schedule does not change. Shared with the interface, in memory only.
pub type RunNow = Arc<std::sync::Mutex<HashSet<String>>>;

/// The browser's headers (cookies, page) of each download the extension
/// sent, by item id: kept in memory only (owner's decision on #7).
pub type FromBrowserById = Arc<std::sync::Mutex<HashMap<u64, crate::FromBrowser>>>;

/// How the download of an item ended.
type Ended = (u64, Result<Finished, Error>);

/// A download the runner started, and how to stop it.
struct Running {
    /// Its task, to know which item a task that panicked was.
    task: tokio::task::Id,
    stop: Stop,
    /// The runner itself asked it to stop: it goes back to waiting.
    by_runner: bool,
    /// From a private window: nothing of it stays if it fails or is taken
    /// out of the queues (review of #91).
    private: bool,
    /// Its own speed limit, its rate read from its item at each look: a
    /// new limit reaches it while it runs (#106).
    limit: SpeedLimit,
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
            from_browser: None,
            run_now: None,
            retry_at: Arc::default(),
        }
    }

    /// A queue in `queues` runs as if its window were open, until it has
    /// nothing waiting or running, when it leaves the set (#125).
    pub fn run_now(mut self, queues: RunNow) -> Self {
        self.run_now = Some(queues);
        self
    }

    /// Each download whose item is in `headers` sends them (the browser's
    /// cookies for its origin, and its page). An item's entry is removed
    /// once it has finished, or is no longer in the queues; a failed item
    /// keeps it, for a retry.
    pub fn from_browser(mut self, headers: FromBrowserById) -> Self {
        self.from_browser = Some(headers);
        self
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
        // Queues said to be done (#142), until they have work again, and
        // the items finished since the last look: a queue is done when one
        // of its downloads finishes, never by an edit (review of #142).
        let mut done_said: HashSet<String> = HashSet::new();
        let mut finished_since: HashSet<u64> = HashSet::new();
        loop {
            if look {
                look = false;
                let store = self.store.clone();
                let queues = blocking(move || store.load()).await?;
                speed_limit.set_rate(queues.settings.speed_limit);
                for (id, run) in &running {
                    if let Some(item) = queues.item(*id) {
                        run.limit.set_rate(item.limit);
                    }
                }
                // The browser's headers of downloads that are done or gone
                // are forgotten; a failed one keeps them for a retry. An id
                // from `next_id` on was added after this load (the app adds
                // the item, then its headers): not gone, just not seen yet
                // (review of #76).
                if let Some(map) = &self.from_browser {
                    let keep: HashSet<u64> = queues
                        .queues
                        .iter()
                        .flat_map(|q| q.items.iter())
                        .filter(|i| i.status != Status::Done)
                        .map(|i| i.id)
                        .collect();
                    map.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .retain(|id, _| *id >= queues.next_id || keep.contains(id));
                }
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
                for queue in &queues.queues {
                    let all_done = !queue.items.is_empty()
                        && queue
                            .items
                            .iter()
                            .all(|i| i.status == Status::Done && !running.contains_key(&i.id));
                    if !all_done {
                        done_said.remove(&queue.name);
                    } else if queue.items.iter().any(|i| finished_since.contains(&i.id))
                        && done_said.insert(queue.name.clone())
                    {
                        on_event(QueueEvent::QueueDone {
                            queue: queue.name.clone(),
                        });
                    }
                }
                finished_since.clear();
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
                    self.finished(
                        joined,
                        &mut running,
                        &mut summary,
                        (&mut finished_now, &mut finished_since),
                        on_event,
                    )
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
        if let Some(set) = &self.run_now {
            // A queue started now leaves the set once it has nothing waiting
            // or running, has no schedule, or is gone (review of #125: one
            // left in it would ignore a schedule set later).
            set.lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|name| {
                    queues.queue(name).is_some_and(|q| {
                        q.schedule.is_some()
                            && q.items
                                .iter()
                                .any(|i| matches!(i.status, Status::Waiting | Status::Active))
                    })
                });
        }
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
            let started_now = self.run_now.as_ref().is_some_and(|set| {
                set.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .contains(&queue.name)
            });
            if started_now {
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
            // A download that needs the browser's cookies again waits for
            // them, taking no room in its queue (M5).
            let kept = self.from_browser.as_ref().map(|map| {
                map.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .keys()
                    .copied()
                    .collect::<HashSet<u64>>()
            });
            let needs_browser = |i: &crate::queue::Item| match (&kept, &i.browser) {
                (Some(kept), Some(source)) => source.cookies && !kept.contains(&i.id),
                _ => false,
            };
            for item in queue.items.iter().filter(|i| {
                i.status == Status::Waiting && !running.contains_key(&i.id) && needs_browser(i)
            }) {
                on_event(QueueEvent::NeedsBrowser {
                    item: item.id,
                    url: item.url.clone(),
                    source: item.browser.clone().expect("checked"),
                });
            }
            // A download tried again waits out its pause (#142).
            let now = self.engine.clock().now();
            let pausing = |i: &crate::queue::Item| {
                let mut retry_at = self.retry_at.lock().unwrap_or_else(|e| e.into_inner());
                match retry_at.get(&i.id) {
                    Some(at) if *at > now => true,
                    Some(_) => {
                        retry_at.remove(&i.id);
                        false
                    }
                    None => false,
                }
            };
            let next: Vec<_> = queue
                .items
                .iter()
                .filter(|i| {
                    i.status == Status::Waiting
                        && !running.contains_key(&i.id)
                        && !needs_browser(i)
                        && !pausing(i)
                })
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
                        item.last_try = Some(crate::state::now_rfc3339());
                        // Started while held: that is the user's OK.
                        if item.needs_ok {
                            item.needs_ok = false;
                            item.ok_given = true;
                        }
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
                let limit = SpeedLimit::new(item.limit, engine.clock());
                let mut engine = engine
                    .with_own_limit(limit.clone())
                    .named(item.name.as_deref());
                // Not yet started by the user, a file that can run code is
                // held, by the names the download itself gets (review of
                // #90: the probe's name could differ): from a page, by the
                // name it is saved under; renamed by the user (#103), when
                // that name can run code and the server's could not.
                let from_page = item.from_page;
                if !item.ok_given && (from_page || item.name.is_some()) {
                    let hold: crate::HoldCheck = Arc::new(move |server: &str, saved: &str| {
                        use crate::category::runs_code;
                        runs_code(saved) && (from_page || !runs_code(server))
                    });
                    engine = engine.holding(hold);
                }
                if let Some(count) = &self.connections {
                    engine = engine.with_connections(count.load(Ordering::Relaxed));
                }
                // The browser's headers kept in memory; else, after a
                // restart, its page alone, from the queues file (the
                // cookies were never written there).
                let kept = self.from_browser.as_ref().and_then(|map| {
                    map.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(&id)
                        .cloned()
                });
                let from = kept.or_else(|| {
                    let source = item.browser.as_ref()?;
                    let referrer = source.referrer.as_deref()?;
                    Some(crate::FromBrowser::new(&item.url, None, Some(referrer)))
                });
                if let Some(from) = from {
                    engine = engine.with_browser(from);
                }
                let categories = self.categories.clone().filter(|_| item.categorize);
                let private = item.private;
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
                                        let chosen = item.category;
                                        let choose = move |name: &str, url: &reqwest::Url| {
                                            // The user's category first (#103);
                                            // one removed since counts as none.
                                            match chosen
                                                .as_deref()
                                                .and_then(|id| categories.get(id))
                                            {
                                                Some(category) => category
                                                    .folder
                                                    .clone()
                                                    .unwrap_or_else(|| given.clone()),
                                                None => categories.folder_for(
                                                    name,
                                                    url.host_str(),
                                                    &given,
                                                ),
                                            }
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
                        private,
                        limit,
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
        (finished_now, finished_since): (&mut HashSet<u64>, &mut HashSet<u64>),
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
        let (by_runner, private) = running
            .remove(&id)
            .map_or((false, false), |run| (run.by_runner, run.private));
        let event = match &result {
            Ok(done) => {
                summary.finished += 1;
                finished_now.insert(id);
                finished_since.insert(id);
                if let Some(map) = &self.from_browser {
                    map.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                }
                QueueEvent::Finished {
                    item: id,
                    file: done.file.clone(),
                }
            }
            // Held for the user is not a failure: it waits, paused.
            Err(Error::Stopped { .. } | Error::Held(_)) => QueueEvent::Stopped { item: id },
            Err(err) => QueueEvent::Failed {
                item: id,
                error: err.to_string(),
            },
        };
        // Only a failure that can pass is tried again (review of #142: a
        // 404, a refused certificate or a full disk would fail the same way
        // at once): a cut, the network, the server's own error.
        let worth_retrying = match &result {
            Err(Error::Http(_) | Error::Interrupted { .. } | Error::NoData(_)) => true,
            Err(Error::Status(status)) => status.is_server_error(),
            _ => false,
        };
        // A private window's download that failed, or was taken out of the
        // queues while it ran (its window closed): it leaves the queues, and
        // its part and state files are deleted (review of #91). Paused by
        // the user, it stays while its window is open.
        if private && result.is_err() {
            let stopped = matches!(result, Err(Error::Stopped { .. }));
            let mut files: Vec<PathBuf> = match &result {
                Err(Error::Stopped {
                    state_file: Some(state),
                })
                | Err(Error::Interrupted {
                    state_file: state, ..
                }) => crate::queue::store::private_files(state),
                _ => Vec::new(),
            };
            let taken = self
                .update(move |queues| match queues.item(id) {
                    Some(_) if stopped => Ok(None),
                    Some(_) => Ok(Some(queues.remove(id)?.state_file)),
                    None => Ok(Some(None)),
                })
                .await;
            let taken = match taken {
                Err(StoreError::NoSuchItem(_)) => Some(None),
                other => other?,
            };
            if let Some(state) = taken {
                files.extend(
                    state
                        .iter()
                        .flat_map(|s| crate::queue::store::private_files(s)),
                );
                blocking(move || {
                    for file in files {
                        let _ = std::fs::remove_file(file);
                    }
                    Ok::<_, StoreError>(())
                })
                .await?;
                if matches!(event, QueueEvent::Failed { .. }) {
                    summary.failed += 1;
                }
                on_event(event);
                return Ok(());
            }
        }
        let tries = ignore_removed_with(
            self.update(move |queues| {
                // Its queue's tries for each download (#142).
                let max_tries = queues
                    .queues
                    .iter()
                    .find(|q| q.items.iter().any(|i| i.id == id))
                    .and_then(|q| q.max_tries)
                    .unwrap_or(1);
                let mut again = false;
                let item = queues.item_mut(id).ok_or(StoreError::NoSuchItem(id))?;
                // A private window's download, done: nothing of it stays in
                // the queues, only its file (security review #84).
                if item.private && result.is_ok() {
                    queues.remove(id)?;
                    return Ok(None);
                }
                match result {
                    Ok(done) => {
                        item.status = Status::Done;
                        item.file = Some(done.file);
                        item.state_file = None;
                        item.error = None;
                        item.failed_tries = 0;
                        // Where it came from is no longer needed.
                        item.browser = None;
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
                    // Held for the user: nothing was made.
                    Err(Error::Held(_)) => {
                        item.status = Status::Paused;
                        item.needs_ok = true;
                        item.error = None;
                    }
                    Err(err) => {
                        match &err {
                            Error::Interrupted { state_file, .. } => {
                                item.state_file = Some(state_file.clone());
                            }
                            // Nothing written, files removed (review of #31).
                            Error::NoData(_) => item.state_file = None,
                            _ => {}
                        }
                        item.error = Some(err.to_string());
                        // With tries left in its queue, it waits again at
                        // the end of the queue (#142); else it failed.
                        item.failed_tries = item.failed_tries.saturating_add(1);
                        again = worth_retrying && item.failed_tries < max_tries;
                        item.status = if again {
                            Status::Waiting
                        } else {
                            Status::Failed
                        };
                    }
                }
                if again
                    && let Some(queue) = queues
                        .queues
                        .iter_mut()
                        .find(|q| q.items.iter().any(|i| i.id == id))
                    && let Some(at) = queue.items.iter().position(|i| i.id == id)
                {
                    let item = queue.items.remove(at);
                    let tries = item.failed_tries;
                    queue.items.push(item);
                    return Ok(Some(tries));
                }
                Ok(None)
            })
            .await,
        )?;
        // Tried again: after a pause that grows with its tries (review of
        // #142), and said as a new try, not as a failure.
        let event = match (event, tries) {
            (QueueEvent::Failed { item, error }, Some(tries)) => {
                let at = self
                    .engine
                    .clock()
                    .now()
                    .saturating_add(crate::retry::backoff(tries));
                self.retry_at
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id, at);
                QueueEvent::Retrying { item, error }
            }
            (event, _) => event,
        };
        if matches!(event, QueueEvent::Failed { .. }) {
            summary.failed += 1;
        }
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

/// [`ignore_removed`], for a change that answers something: the default.
fn ignore_removed_with<T: Default>(result: Result<T, StoreError>) -> Result<T, StoreError> {
    match result {
        Err(StoreError::NoSuchItem(_)) => Ok(T::default()),
        other => other,
    }
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
