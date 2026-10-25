//! `zajil queue`: managing and running the queues (M3, ADR-0007).
//!
//! Every command changes the queues file only through
//! [`QueueStore::update`], so it can run while `zajil queue run` runs: the
//! runner sees the change at its next look (twice a second).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use std::time::Duration;

use zajil_core::limit::Clock;
use zajil_core::queue::{
    AfterQueue, DEFAULT_QUEUE, Day, Item, MAX_ACTIVE, QueueEvent, QueueStore, Runner, Schedule,
    ScheduleError, Status, StoreError, Summary, TimeOfDay, after_queue, shown_url,
};
use zajil_core::state::{DownloadState, STATE_SUFFIX};
use zajil_core::{Engine, Event, Stop};
use zajil_platform::power::{PowerAction, SystemPower};

use crate::{EXIT_RESUMABLE, describe, fail, human_bytes};

#[derive(Debug, Subcommand)]
pub enum QueueCommand {
    /// Add downloads to the end of a queue.
    Add {
        /// The http:// or https:// URLs. A URL with a user name or password
        /// cannot be queued (Zajil never saves them).
        #[arg(required = true)]
        urls: Vec<String>,
        /// The queue (created if needed).
        #[arg(short, long, value_name = "NAME", default_value = DEFAULT_QUEUE)]
        queue: String,
        /// Folder to save into. Without it, each file goes to its
        /// category's folder (see `zajil category`), or to the system
        /// Downloads folder.
        #[arg(short, long, value_name = "FOLDER")]
        dir: Option<PathBuf>,
        /// With --dir: each file still goes to its category's folder, and
        /// to --dir only when its category has no folder.
        #[arg(long, requires = "dir")]
        by_category: bool,
        /// A speed limit for each of these downloads, like `zajil get
        /// --limit`. The limit for the whole program applies too.
        #[arg(long, value_name = "RATE", value_parser = crate::rate)]
        limit: Option<u64>,
    },
    /// Show the queues and their downloads.
    List,
    /// Pause downloads; a running one stops after recording its progress.
    Pause {
        #[arg(required = true, value_name = "ID")]
        ids: Vec<u64>,
    },
    /// Put paused or failed downloads back in their queue.
    Resume {
        #[arg(required = true, value_name = "ID")]
        ids: Vec<u64>,
    },
    /// Remove downloads from their queue. A running one stops; files
    /// already downloaded stay where they are.
    Remove {
        #[arg(required = true, value_name = "ID")]
        ids: Vec<u64>,
    },
    /// Change the settings of a queue.
    Set {
        /// The queue (created if needed).
        #[arg(short, long, value_name = "NAME", default_value = DEFAULT_QUEUE)]
        queue: String,
        /// How many of its downloads run at once.
        #[arg(
            long,
            value_name = "N",
            value_parser = clap::value_parser!(u32).range(1..=MAX_ACTIVE as i64)
        )]
        max_active: Option<u32>,
    },
    /// Set or remove the speed limit for all downloads together.
    SpeedLimit {
        /// Bytes per second, or with K, M or G (such as 2M); `none` removes
        /// the limit. A running `zajil queue run` applies it within a
        /// second.
        #[arg(value_name = "RATE|none", value_parser = limit_or_none)]
        rate: LimitSetting,
    },
    /// Let a queue run only in a window each day (local time). Outside it,
    /// its downloads stop after recording their progress, and resume at
    /// the next window.
    Schedule {
        /// The queue (created if needed).
        #[arg(short, long, value_name = "NAME", default_value = DEFAULT_QUEUE)]
        queue: String,
        /// When the window begins, HH:MM or HH:MM:SS (24-hour). Default
        /// 00:00.
        #[arg(long, value_name = "TIME", value_parser = time_of_day)]
        start: Option<TimeOfDay>,
        /// When it ends; if not after the start, it ends the next day.
        /// Default: midnight.
        #[arg(long, value_name = "TIME", value_parser = time_of_day)]
        stop: Option<TimeOfDay>,
        /// The days a window begins on, such as sat,sun. Default: every
        /// day.
        #[arg(long, value_name = "DAYS", value_delimiter = ',', value_parser = day)]
        days: Vec<Day>,
        /// Remove the schedule: the queue runs at any time.
        #[arg(long, conflicts_with_all = ["start", "stop", "days"])]
        none: bool,
    },
    /// Save the queues, their settings and their downloads in a file, to
    /// import them elsewhere. Progress is not saved. The file is readable
    /// by you only (a URL may hold a token), and never written over.
    Export {
        /// The file to create.
        file: PathBuf,
    },
    /// Add the queues and downloads of an exported file. Queues that
    /// exist keep their settings and get the downloads at their end.
    Import {
        /// The exported file.
        file: PathBuf,
        /// Save into the folders written in the file. Without it, the
        /// downloads go to --dir (or the system Downloads folder) and their
        /// categories' folders: a file from someone else could otherwise
        /// send downloads anywhere.
        #[arg(long, conflicts_with = "dir")]
        keep_folders: bool,
        /// The folder for the imported downloads.
        #[arg(short, long, value_name = "FOLDER")]
        dir: Option<PathBuf>,
    },
    /// Run every queue until nothing is left to download. Ctrl+C stops the
    /// downloads after recording their progress; run it again to go on.
    Run {
        /// When this queue has finished (every download in it done, at
        /// least one in this run), stop the other downloads and shut the
        /// computer down, after a countdown that Ctrl+C cancels. If it
        /// cannot finish (a download failed or is paused), the run goes on
        /// and the computer stays on. Asked for this run only; never saved.
        #[arg(long, value_name = "QUEUE", conflicts_with = "sleep_after")]
        shutdown_after: Option<String>,
        /// The same, but put the computer to sleep.
        #[arg(long, value_name = "QUEUE")]
        sleep_after: Option<String>,
        /// Seconds of the countdown before shutting down or sleeping.
        #[arg(
            long,
            value_name = "SECONDS",
            default_value_t = 60,
            value_parser = clap::value_parser!(u64).range(10..=3600)
        )]
        countdown: u64,
    },
}

/// The commands that only change or read the queues file.
pub fn manage(store: &QueueStore, command: QueueCommand) -> ExitCode {
    let result: Result<(), String> = match command {
        QueueCommand::Export { file } => export(store, &file),
        QueueCommand::Import {
            file,
            keep_folders,
            dir,
        } => import(store, &file, keep_folders, dir.as_deref()),
        QueueCommand::Schedule {
            queue,
            start,
            stop,
            days,
            none,
        } => schedule(store, &queue, (!none).then_some((start, stop, days))),
        other => manage_queues(store, other).map_err(|err| err.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => fail(&message),
    }
}

fn manage_queues(store: &QueueStore, command: QueueCommand) -> Result<(), StoreError> {
    match command {
        QueueCommand::Add {
            urls,
            queue,
            dir,
            by_category,
            limit,
        } => add(store, &urls, &queue, dir.as_deref(), by_category, limit),
        QueueCommand::SpeedLimit { rate } => speed_limit(store, rate.0),
        QueueCommand::List => list(store),
        QueueCommand::Pause { ids } => set_status(store, &ids, Status::Paused, "paused"),
        QueueCommand::Resume { ids } => set_status(store, &ids, Status::Waiting, "waiting"),
        QueueCommand::Remove { ids } => remove(store, &ids),
        QueueCommand::Set { queue, max_active } => set(store, &queue, max_active),
        QueueCommand::Run { .. }
        | QueueCommand::Schedule { .. }
        | QueueCommand::Export { .. }
        | QueueCommand::Import { .. } => {
            unreachable!("handled before")
        }
    }
}

/// A rate, or no limit.
#[derive(Debug, Clone, Copy)]
pub struct LimitSetting(Option<u64>);

fn limit_or_none(text: &str) -> Result<LimitSetting, String> {
    if text.eq_ignore_ascii_case("none") {
        Ok(LimitSetting(None))
    } else {
        crate::rate(text).map(|rate| LimitSetting(Some(rate)))
    }
}

fn add(
    store: &QueueStore,
    urls: &[String],
    queue: &str,
    dir: Option<&Path>,
    by_category: bool,
    limit: Option<u64>,
) -> Result<(), StoreError> {
    // Without a folder, each download goes by its category (M4).
    let categorize = dir.is_none() || by_category;
    let dir = crate::download_dir(dir).map_err(|message| StoreError::Io(io_error(message)))?;
    let ids = store.update(|queues| {
        urls.iter()
            .map(|url| {
                let id = queues.add(queue, url, &dir)?;
                let item = queues.item_mut(id).expect("just added");
                item.limit = limit;
                item.categorize = categorize;
                Ok(id)
            })
            .collect::<Result<Vec<_>, _>>()
    })?;
    for (id, url) in ids.iter().zip(urls) {
        println!("Added {id}: {}", shown_url(url));
    }
    Ok(())
}

fn export(store: &QueueStore, file: &Path) -> Result<(), String> {
    let queues = store.load().map_err(|err| err.to_string())?;
    let export = queues.export();
    zajil_core::queue::write_export(file, &export).map_err(|err| err.to_string())?;
    let items: usize = export.queues.iter().map(|q| q.items.len()).sum();
    println!(
        "Exported {} queue(s) and {items} download(s) to {}",
        export.queues.len(),
        file.display()
    );
    // Owner's decision on #41: say it, without printing any URL.
    eprintln!(
        "zajil: note: the file holds the URLs as they are, and some may contain temporary \
         access keys. Do not share it."
    );
    Ok(())
}

fn import(
    store: &QueueStore,
    file: &Path,
    keep_folders: bool,
    dir: Option<&Path>,
) -> Result<(), String> {
    // Everything is checked before the queues file is touched.
    let export = zajil_core::queue::read_export(file).map_err(|err| err.to_string())?;
    let folders = if keep_folders {
        zajil_core::queue::Folders::Keep
    } else {
        // Created only once the import is done: a refused import leaves
        // nothing behind (review of #41, round 3).
        zajil_core::queue::Folders::Into(crate::download_dir_path(dir)?)
    };
    let summary = store
        .update(|queues| queues.import(&export, &folders))
        .map_err(|err| err.to_string())?;

    println!(
        "Imported {} download(s) ({} new queue(s)) from {}",
        summary.items_added,
        summary.queues_created,
        file.display()
    );
    for name in &summary.kept_settings {
        println!("Queue {name} already had settings of its own: it kept them");
    }
    if summary.speed_limit_set {
        println!(
            "{}",
            describe_limit(
                store
                    .load()
                    .map_err(|e| e.to_string())?
                    .settings
                    .speed_limit
            )
        );
    }
    // The import is saved: a folder that cannot be made now is a warning,
    // not a failure, or running the command again would add everything
    // twice (review of #41, round 4). The download makes it when it starts.
    if let zajil_core::queue::Folders::Into(dir) = &folders
        && let Err(err) = std::fs::create_dir_all(dir)
    {
        eprintln!(
            "zajil: warning: cannot create the folder {} ({err}); the downloads were added, and \
             will fail until it exists",
            dir.display()
        );
    }
    Ok(())
}

fn speed_limit(store: &QueueStore, rate: Option<u64>) -> Result<(), StoreError> {
    store.update(|queues| {
        queues.settings.speed_limit = rate;
        Ok(())
    })?;
    println!("{}", describe_limit(rate));
    Ok(())
}

fn time_of_day(text: &str) -> Result<TimeOfDay, String> {
    text.parse().map_err(|err: ScheduleError| err.to_string())
}

fn day(text: &str) -> Result<Day, String> {
    text.parse().map_err(|err: ScheduleError| err.to_string())
}

/// Sets the schedule of `queue`, or removes it (`None`).
fn schedule(
    store: &QueueStore,
    queue: &str,
    window: Option<(Option<TimeOfDay>, Option<TimeOfDay>, Vec<Day>)>,
) -> Result<(), String> {
    let schedule = match window {
        Some((start, stop, days)) => Some(
            Schedule::new(start, stop, days)
                .map_err(|_| "give --start, --stop or --days, or --none".to_owned())?,
        ),
        None => None,
    };
    store
        .update(|queues| {
            queues.queue_or_new(queue)?.schedule = schedule.clone();
            Ok(())
        })
        .map_err(|err| err.to_string())?;
    match schedule {
        Some(schedule) => println!("Queue {queue} runs from {schedule} (local time)"),
        None => println!("Queue {queue} runs at any time"),
    }
    Ok(())
}

fn describe_limit(rate: Option<u64>) -> String {
    match rate {
        Some(rate) => format!("Speed limit for all downloads: {}/s", human_bytes(rate)),
        None => "No speed limit for all downloads".to_owned(),
    }
}

fn list(store: &QueueStore) -> Result<(), StoreError> {
    let queues = store.load()?;
    println!("{}", describe_limit(queues.settings.speed_limit));
    for queue in &queues.queues {
        println!(
            "Queue {} ({} at once, {} downloads)",
            queue.name,
            queue.max_active,
            queue.items.len()
        );
        if let Some(schedule) = &queue.schedule {
            println!("  runs from {schedule} (local time)");
        }
        for item in &queue.items {
            let limit = item
                .limit
                .map(|rate| format!("  (limit {}/s)", human_bytes(rate)))
                .unwrap_or_default();
            println!(
                "  {:>4}  {:<8} {:>6}  {}{limit}",
                item.id,
                status_name(item.status),
                progress(item),
                label(item)
            );
            if let Some(error) = item
                .error
                .as_deref()
                .filter(|_| item.status == Status::Failed)
            {
                println!("        {error}");
            }
        }
    }
    Ok(())
}

fn set_status(
    store: &QueueStore,
    ids: &[u64],
    status: Status,
    shown: &str,
) -> Result<(), StoreError> {
    store.update(|queues| {
        for &id in ids {
            let item = queues.item_mut(id).ok_or(StoreError::NoSuchItem(id))?;
            // Only what can change: a finished download stays finished, and
            // resuming applies to paused or failed ones.
            let allowed = match status {
                Status::Paused => matches!(item.status, Status::Waiting | Status::Active),
                _ => matches!(item.status, Status::Paused | Status::Failed),
            };
            if allowed {
                item.status = status;
                // Started by the user: its queue's tries count from 0 (#142).
                if status == Status::Waiting {
                    item.failed_tries = 0;
                }
            }
        }
        Ok(())
    })?;
    let queues = store.load()?;
    for &id in ids {
        if let Some(item) = queues.item(id) {
            let now = status_name(item.status);
            if now == shown {
                println!("{id} is {now}");
            } else {
                println!("{id} stays {now}");
            }
        }
    }
    Ok(())
}

fn remove(store: &QueueStore, ids: &[u64]) -> Result<(), StoreError> {
    let removed = store.update(|queues| {
        ids.iter()
            .map(|&id| queues.remove(id))
            .collect::<Result<Vec<_>, _>>()
    })?;
    for item in removed {
        println!("Removed {}: {}", item.id, label(&item));
    }
    Ok(())
}

fn set(store: &QueueStore, queue: &str, max_active: Option<u32>) -> Result<(), StoreError> {
    store.update(|queues| {
        let queue = queues.queue_or_new(queue)?;
        if let Some(max_active) = max_active {
            queue.max_active = max_active;
        }
        Ok(())
    })?;
    let queues = store.load()?;
    if let Some(queue) = queues.queue(queue) {
        println!("Queue {}: {} at once", queue.name, queue.max_active);
    }
    Ok(())
}

/// What `zajil queue run` does with the computer after a queue, if the
/// user asked for it.
pub struct PowerRequest {
    pub queue: String,
    pub action: PowerAction,
    pub countdown: Duration,
}

impl PowerRequest {
    pub fn from_flags(
        shutdown_after: Option<String>,
        sleep_after: Option<String>,
        countdown: u64,
    ) -> Option<Self> {
        let (queue, action) = match (shutdown_after, sleep_after) {
            (Some(queue), _) => (queue, PowerAction::Shutdown),
            (None, Some(queue)) => (queue, PowerAction::Sleep),
            (None, None) => return None,
        };
        Some(PowerRequest {
            queue,
            action,
            countdown: Duration::from_secs(countdown),
        })
    }
}

/// `zajil queue run`.
pub async fn run(
    store: QueueStore,
    categories: zajil_core::category::CategoryStore,
    engine: Engine,
    power: Option<PowerRequest>,
) -> ExitCode {
    let stop = Stop::new();
    let on_ctrl_c = stop.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("zajil: stopping (Ctrl+C)");
            on_ctrl_c.request();
        }
    });
    if let Some(request) = &power
        && let Err(err) = store.load().and_then(|queues| {
            queues
                .queue(&request.queue)
                .map(|_| ())
                .ok_or_else(|| StoreError::NoSuchQueue(request.queue.clone()))
        })
    {
        return fail(&err.to_string());
    }
    let clock = engine.clock();
    let mut runner = Runner::new(engine, store.clone()).categories(categories);
    if let Some(request) = &power {
        eprintln!(
            "zajil: the computer will {} when queue {} has finished (after a countdown)",
            request.action, request.queue
        );
        runner = runner.until_queue_done(&request.queue);
    }
    let result = runner.run(&stop, &mut |event| show(event)).await;
    if let (Some(request), Ok(summary)) = (&power, &result) {
        return power_after(&store, request, summary, clock.as_ref(), &stop).await;
    }
    match result {
        Ok(summary) if summary.stopped => {
            eprintln!("zajil: stopped; run `zajil queue run` again to go on");
            ExitCode::from(EXIT_RESUMABLE)
        }
        Ok(summary) if summary.failed > 0 => {
            eprintln!(
                "zajil: {} finished, {} failed (see `zajil queue list`)",
                summary.finished, summary.failed
            );
            ExitCode::FAILURE
        }
        Ok(summary) => {
            eprintln!(
                "zajil: {} finished; nothing is left to download",
                summary.finished
            );
            ExitCode::SUCCESS
        }
        Err(err) => fail(&err.to_string()),
    }
}

/// The countdown and the action after the queue, or why there is none.
async fn power_after(
    store: &QueueStore,
    request: &PowerRequest,
    summary: &Summary,
    clock: &dyn Clock,
    cancel: &Stop,
) -> ExitCode {
    if summary.stopped {
        eprintln!("zajil: stopped; the computer stays on");
        return ExitCode::from(EXIT_RESUMABLE);
    }
    let action = request.action;
    let mut announced = false;
    let outcome = after_queue(
        store,
        &request.queue,
        summary,
        &SystemPower,
        action,
        request.countdown,
        clock,
        cancel,
        &mut |left| {
            // Whole seconds left, rounded up: the first line says the full
            // countdown. Then every 10 seconds, and each of the last 5.
            let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
            let first = std::mem::replace(&mut announced, true);
            if !first || secs % 10 == 0 || secs <= 5 {
                eprintln!(
                    "zajil: queue {} has finished: the computer will {action} in {secs} s; \
                     press Ctrl+C to cancel",
                    request.queue
                );
            }
        },
    )
    .await;
    match outcome {
        Ok(AfterQueue::Done) => {
            eprintln!("zajil: asked the system to {action}");
            ExitCode::SUCCESS
        }
        Ok(AfterQueue::Cancelled) => {
            eprintln!("zajil: cancelled; the computer stays on");
            ExitCode::SUCCESS
        }
        Ok(AfterQueue::NotFinished { left }) => fail(&format!(
            "queue {} did not finish ({left} download(s) not done, see `zajil queue list`); \
             the computer stays on",
            request.queue
        )),
        Ok(AfterQueue::Failed(err)) => fail(&format!("could not {action}: {err}")),
        Err(err) => fail(&err.to_string()),
    }
}

fn show(event: QueueEvent) {
    match event {
        QueueEvent::Started { queue, item } => eprintln!("[{item}] starting (queue {queue})"),
        QueueEvent::Download { item, event } => match event {
            Event::Started {
                file, total, from, ..
            } => {
                let size = total
                    .map(human_bytes)
                    .unwrap_or_else(|| "unknown size".into());
                if from > 0 {
                    eprintln!(
                        "[{item}] resuming {} at {} ({size})",
                        file.display(),
                        human_bytes(from)
                    );
                } else {
                    eprintln!("[{item}] downloading {} ({size})", file.display());
                }
            }
            Event::Restarted { reason } => {
                eprintln!("[{item}] starting over: {}", describe(reason))
            }
            Event::Retrying { wait, reason } => eprintln!(
                "[{item}] trying again in {} s: {reason}",
                wait.as_secs_f64().ceil()
            ),
            Event::ServerBusy {
                status,
                connections,
                ..
            } => eprintln!(
                "[{item}] the server answered {status}: {connections} connection(s) from now on"
            ),
            Event::Progress { .. } | Event::Split { .. } | Event::Finished { .. } => {}
        },
        QueueEvent::Finished { item, file } => eprintln!("[{item}] saved {}", file.display()),
        QueueEvent::Failed { item, error } => eprintln!("[{item}] failed: {error}"),
        QueueEvent::Stopped { item } => eprintln!("[{item}] stopped"),
        // Only the app keeps the browser's cookies: this runner has none to
        // wait for, so it never says this.
        QueueEvent::NeedsBrowser { item, .. } => {
            eprintln!("[{item}] waiting for the browser's cookies (open the browser)")
        }
        QueueEvent::QueueDone { queue } => eprintln!("queue {queue} is done"),
        QueueEvent::Retrying { item, error } => {
            eprintln!("[{item}] failed: {error}; its queue tries it again later")
        }
        QueueEvent::Waiting { queue, until } => eprintln!(
            "queue {queue} waits for its schedule until {} (local time)",
            until.strftime("%Y-%m-%d %H:%M:%S")
        ),
    }
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Waiting => "waiting",
        Status::Active => "active",
        Status::Paused => "paused",
        Status::Done => "done",
        Status::Failed => "failed",
    }
}

/// How much of the item is on disk, from its own state file.
fn progress(item: &Item) -> String {
    if item.status == Status::Done {
        return "100%".into();
    }
    let state = item
        .state_file
        .as_deref()
        .and_then(|path| DownloadState::load(path).ok());
    match state {
        Some(state) => match state.content_length {
            Some(total) if total > 0 => {
                format!("{:.0}%", state.bytes_done() as f64 * 100.0 / total as f64)
            }
            _ => human_bytes(state.bytes_done()),
        },
        None => "-".into(),
    }
}

/// The file's name once known, otherwise the URL without its secrets.
fn label(item: &Item) -> String {
    let named = item.file.clone().or_else(|| {
        item.state_file.as_deref().and_then(|path| {
            let name = path.file_name()?.to_str()?.strip_suffix(STATE_SUFFIX)?;
            Some(PathBuf::from(name))
        })
    });
    match named {
        Some(path) => path.display().to_string(),
        None => shown_url(&item.url),
    }
}

fn io_error(message: String) -> std::io::Error {
    std::io::Error::other(message)
}
