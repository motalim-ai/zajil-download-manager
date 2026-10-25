//! Queues (M3, ADR-0007): named lists of downloads, each with a number of
//! downloads that run at once, kept in `queues.json` across restarts.

mod power;
mod runner;
mod schedule;
mod store;
mod transfer;

pub use power::{AfterQueue, after_queue};
pub use runner::{FromBrowserById, QueueEvent, Runner, Summary};
pub use schedule::{Day, Schedule, ScheduleError, TimeOfDay};
pub use store::{
    DEFAULT_MAX_ACTIVE, DEFAULT_QUEUE, FORMAT, Item, MAX_ACTIVE, MAX_NAME_CHARS, Queue, QueueStore,
    QueuesFile, RunnerLock, SCHEMA_VERSION, Settings, Status, StoreError, check_name, check_url,
    shown_url,
};
pub use transfer::{
    EXPORT_FORMAT, EXPORT_SCHEMA_VERSION, ExportFile, ExportItem, ExportQueue, ExportStatus,
    Folders, ImportSummary, MAX_EXPORT_BYTES, TransferError, read_export, write_export,
};
