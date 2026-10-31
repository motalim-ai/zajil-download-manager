//! Zajil download engine.
//!
//! A UI-less library: it takes commands and reports events, and never prints
//! or reads user settings (see `docs/ARCHITECTURE.md`).

#![forbid(unsafe_code)]

pub mod browser;
pub mod category;
pub mod clipboard;
pub mod drive;
mod engine;
mod http_date;
pub mod limit;
pub mod naming;
pub mod queue;
pub mod relocate;
mod retry;
pub mod sanitize;
mod segmented;
pub mod settings;
pub mod sites;
pub mod state;
pub mod tools;
pub mod tools_list;
pub mod update;
pub mod video;

pub use browser::FromBrowser;
#[cfg(feature = "test-support")]
pub use engine::engine_test_hooks;
pub use engine::{
    Engine, Error, Event, FetchedPage, Finished, HoldCheck, Mismatch, Options, Probe,
    RestartReason, Stop, is_page_type,
};
/// URLs as the engine parses them (for [`Engine::download_choosing_until`]).
pub use reqwest::Url;
pub use segmented::{DEFAULT_CONNECTIONS, MAX_CONNECTIONS};

/// How many times in a row a failing request or connection is tried by
/// default ([`Options::max_failures`]).
pub const DEFAULT_TRIES: u32 = retry::MAX_FAILURES;

/// Zajil's version: the workspace's, the one its releases carry (#248: the
/// desktop crate's own is always 0.0.0).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
