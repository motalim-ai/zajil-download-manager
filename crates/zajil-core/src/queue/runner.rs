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

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::store::{QueueStore, QueuesFile, Status, StoreError};
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
    /// The download was stopped (paused, removed, or the runner stops).
    Stopped {
        item: u64,
    },
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Summary {
    pub finished: usize,
    pub failed: usize,
    /// The run was asked to stop before the queues were done.
    pub stopped: bool,
}

/// Runs every queue of one [`QueueStore`].
#[derive(Debug, Clone)]
pub struct Runner {
    engine: Engine,
    store: QueueStore,
    poll: Duration,
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
        }
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

        let (events, mut messages) = mpsc::unbounded_channel::<(u64, Event)>();
        let mut tasks: JoinSet<Ended> = JoinSet::new();
        let mut running: HashMap<u64, Running> = HashMap::new();
        let mut summary = Summary::default();
        let mut tick = tokio::time::interval(self.poll);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Whether to read the queues file again: at each tick and after
        // each change, but not for every progress event.
        let mut look = true;
        loop {
            if look {
                look = false;
                let store = self.store.clone();
                let queues = blocking(move || store.load()).await?;
                self.stop_unwanted(&queues, &mut running);
                if !summary.stopped {
                    self.start_waiting(&queues, &mut running, &mut tasks, &events, on_event)
                        .await?;
                }
                if running.is_empty() && (summary.stopped || !has_waiting(&queues)) {
                    return Ok(summary);
                }
            }
            tokio::select! {
                Some(joined) = tasks.join_next_with_id() => {
                    // Events the task sent before it ended come first: a
                    // late `Started` must not record a state file after the
                    // end was recorded (review of #31).
                    while let Ok((item, event)) = messages.try_recv() {
                        self.download_event(item, event, &running, on_event).await?;
                    }
                    self.finished(joined, &mut running, &mut summary, on_event).await?;
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
            }
        }
    }

    /// Stops the downloads whose items the user paused or removed (or set
    /// to anything but active).
    fn stop_unwanted(&self, queues: &QueuesFile, running: &mut HashMap<u64, Running>) {
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
    async fn start_waiting(
        &self,
        queues: &QueuesFile,
        running: &mut HashMap<u64, Running>,
        tasks: &mut JoinSet<Ended>,
        events: &mpsc::UnboundedSender<(u64, Event)>,
        on_event: &mut (dyn FnMut(QueueEvent) + Send),
    ) -> Result<(), StoreError> {
        for queue in &queues.queues {
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
                let engine = self.engine.clone();
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
                        None => {
                            engine
                                .download_until(&item.url, &item.dir, &stop, &mut report)
                                .await
                        }
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
