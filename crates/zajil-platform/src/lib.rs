//! Zajil platform layer.
//!
//! Everything that differs between Windows, macOS and Linux lives behind a
//! trait in this crate, with one implementation per OS chosen at build time
//! (ADR-0006). Code outside this crate must not use `cfg(target_os)`.
//!
//! Phase M1 provides [`AppPaths`] and the fixed identifiers from ADR-0001.

#![forbid(unsafe_code)]

pub mod ids;
mod paths;

pub use paths::{AppPaths, PathsError, SystemPaths, linux, macos, windows};
