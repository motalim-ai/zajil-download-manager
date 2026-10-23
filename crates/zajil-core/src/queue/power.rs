//! Shutting down or sleeping after a queue finishes (M3, `PowerControl`).
//!
//! The owner's safety rules:
//! - **Only when the user asked, for that queue**: nothing here runs unless
//!   an interface calls [`after_queue`] for a queue the user named, and the
//!   runner was told to end with that queue (`Runner::until_queue_done`).
//!   The request is not saved anywhere, so it cannot surprise anyone later.
//! - **Only if the queue really finished**: every one of its downloads is
//!   done. A failure, a pause, or a run the user stopped means no action.
//! - **A clear countdown the user can cancel**, before anything happens.
//! - Tests use a mock `PowerControl`, never the real one.

use std::io;
use std::time::Duration;

use zajil_platform::power::{PowerAction, PowerControl};

use super::runner::Summary;
use super::store::{QueueStore, Status, StoreError};
use crate::Stop;
use crate::limit::Clock;

/// How [`after_queue`] ended.
#[derive(Debug)]
pub enum AfterQueue {
    /// The system was asked for the action.
    Done,
    /// The user cancelled the countdown.
    Cancelled,
    /// The queue did not finish (or the run did not end with it): nothing
    /// was done. `left` of its downloads are not done.
    NotFinished { left: usize },
    /// The system refused or failed.
    Failed(io::Error),
}

/// After a run that ended with `queue` (`Runner::until_queue_done`): if
/// every download of `queue` is done, counts `countdown` down on `clock`
/// (`on_tick` hears the time left, once a second), then asks `power` for
/// `action`. `cancel` stops the countdown at any moment.
#[allow(clippy::too_many_arguments)]
pub async fn after_queue(
    store: &QueueStore,
    queue: &str,
    summary: &Summary,
    power: &dyn PowerControl,
    action: PowerAction,
    countdown: Duration,
    clock: &dyn Clock,
    cancel: &Stop,
    on_tick: &mut (dyn FnMut(Duration) + Send),
) -> Result<AfterQueue, StoreError> {
    let left = {
        let store = store.clone();
        let queue = queue.to_owned();
        tokio::task::spawn_blocking(move || {
            let queues = store.load()?;
            let items = &queues
                .queue(&queue)
                .ok_or(StoreError::NoSuchQueue(queue.clone()))?
                .items;
            Ok::<_, StoreError>(if items.is_empty() {
                // Nothing finished: an empty queue never shuts anything down.
                1
            } else {
                items.iter().filter(|i| i.status != Status::Done).count()
            })
        })
        .await
        .map_err(|err| StoreError::Io(io::Error::other(err)))??
    };
    if !summary.queue_ended || summary.stopped || left > 0 {
        return Ok(AfterQueue::NotFinished { left });
    }

    let end = clock.now().saturating_add(countdown);
    loop {
        let now = clock.now();
        if now >= end {
            break;
        }
        let left = end - now;
        on_tick(left);
        // The next whole second of the countdown (or its end).
        let step = Duration::from_secs(1).min(left);
        tokio::select! {
            () = clock.sleep_until(now + step) => {}
            () = cancel.requested() => return Ok(AfterQueue::Cancelled),
        }
    }
    if cancel.is_requested() {
        return Ok(AfterQueue::Cancelled);
    }
    Ok(match power.perform(action) {
        Ok(()) => AfterQueue::Done,
        Err(err) => AfterQueue::Failed(err),
    })
}
