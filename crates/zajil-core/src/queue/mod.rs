//! Queues (M3, ADR-0007): named lists of downloads, each with a number of
//! downloads that run at once, kept in `queues.json` across restarts.

mod runner;
mod schedule;
mod store;

pub use runner::{QueueEvent, Runner, Summary};
pub use schedule::{Day, Schedule, ScheduleError, TimeOfDay};
pub use store::{
    DEFAULT_MAX_ACTIVE, DEFAULT_QUEUE, FORMAT, Item, MAX_ACTIVE, MAX_NAME_CHARS, Queue, QueueStore,
    QueuesFile, RunnerLock, SCHEMA_VERSION, Status, StoreError, check_name, shown_url,
};
