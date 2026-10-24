//! Zajil download engine.
//!
//! A UI-less library: it takes commands and reports events, and never prints
//! or reads user settings (see `docs/ARCHITECTURE.md`).

#![forbid(unsafe_code)]

pub mod category;
mod engine;
mod http_date;
pub mod limit;
pub mod naming;
pub mod queue;
mod retry;
pub mod sanitize;
mod segmented;
pub mod state;

pub use engine::{Engine, Error, Event, Finished, Options, RestartReason, Stop};
/// URLs as the engine parses them (for [`Engine::download_choosing_until`]).
pub use reqwest::Url;
pub use segmented::{DEFAULT_CONNECTIONS, MAX_CONNECTIONS};

/// How many times in a row a failing request or connection is tried by
/// default ([`Options::max_failures`]).
pub const DEFAULT_TRIES: u32 = retry::MAX_FAILURES;
