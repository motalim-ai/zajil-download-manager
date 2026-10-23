//! Zajil download engine.
//!
//! A UI-less library: it takes commands and reports events, and never prints
//! or reads user settings (see `docs/ARCHITECTURE.md`).

#![forbid(unsafe_code)]

mod engine;
pub mod naming;
pub mod sanitize;
pub mod state;

pub use engine::{Engine, Error, Event, Finished, Options};
