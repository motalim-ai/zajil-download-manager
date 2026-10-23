//! Zajil download engine.
//!
//! A UI-less library: it takes commands and reports events, and never prints
//! or reads user settings (see `docs/ARCHITECTURE.md`).

#![forbid(unsafe_code)]

mod engine;
mod http_date;
pub mod naming;
mod retry;
pub mod sanitize;
mod segmented;
pub mod state;

pub use engine::{Engine, Error, Event, Finished, Options, RestartReason};
pub use segmented::{DEFAULT_CONNECTIONS, MAX_CONNECTIONS};
