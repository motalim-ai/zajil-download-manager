//! `zajil queue`: managing and running the queues (M3, ADR-0007).
//!
//! Every command changes the queues file only through
//! [`QueueStore::update`], so it can run while `zajil queue run` runs: the
//! runner sees the change at its next look (twice a second).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use zajil_core::queue::{
    DEFAULT_QUEUE, Item, MAX_ACTIVE, QueueEvent, QueueStore, Runner, Status, StoreError, shown_url,
};
use zajil_core::state::{DownloadState, STATE_SUFFIX};
use zajil_core::{Engine, Event, Stop};

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
        /// Folder to save into. Defaults to the system Downloads folder.
        #[arg(short, long, value_name = "FOLDER")]
        dir: Option<PathBuf>,
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
    /// Run every queue until nothing is left to download. Ctrl+C stops the
    /// downloads after recording their progress; run it again to go on.
    Run,
}

/// The commands that only change or read the queues file.
pub fn manage(store: &QueueStore, command: QueueCommand) -> ExitCode {
    let result = match command {
        QueueCommand::Add {
            urls,
            queue,
            dir,
            limit,
        } => add(store, &urls, &queue, dir.as_deref(), limit),
        QueueCommand::SpeedLimit { rate } => speed_limit(store, rate.0),
        QueueCommand::List => list(store),
        QueueCommand::Pause { ids } => set_status(store, &ids, Status::Paused, "paused"),
        QueueCommand::Resume { ids } => set_status(store, &ids, Status::Waiting, "waiting"),
        QueueCommand::Remove { ids } => remove(store, &ids),
        QueueCommand::Set { queue, max_active } => set(store, &queue, max_active),
        QueueCommand::Run => unreachable!("run is async"),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => fail(&err.to_string()),
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
    limit: Option<u64>,
) -> Result<(), StoreError> {
    let dir = crate::download_dir(dir).map_err(|message| StoreError::Io(io_error(message)))?;
    let ids = store.update(|queues| {
        urls.iter()
            .map(|url| {
                let id = queues.add(queue, url, &dir)?;
                queues.item_mut(id).expect("just added").limit = limit;
                Ok(id)
            })
            .collect::<Result<Vec<_>, _>>()
    })?;
    for (id, url) in ids.iter().zip(urls) {
        println!("Added {id}: {}", shown_url(url));
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

/// `zajil queue run`.
pub async fn run(store: QueueStore, engine: Engine) -> ExitCode {
    let stop = Stop::new();
    let on_ctrl_c = stop.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("zajil: stopping; the downloads record their progress first");
            on_ctrl_c.request();
        }
    });
    let result = Runner::new(engine, store)
        .run(&stop, &mut |event| show(event))
        .await;
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
