//! Zajil platform layer.
//!
//! Everything that differs between Windows, macOS and Linux lives behind a
//! trait in this crate, with one implementation per OS chosen at build time
//! (ADR-0006). Code outside this crate must not use `cfg(target_os)`.
//!
//! Phase M1 provides [`AppPaths`], the fixed identifiers from ADR-0001 and
//! the [`fs`] helpers for the safe-write rules of ADR-0004.
//!
//! `unsafe` is denied everywhere except the small, commented system calls in
//! [`fs`] and [`console`] that the standard library does not offer
//! (no-replace and write-through renames, hard-link counts, space
//! reservation, sparse files, free space, files only their owner may read,
//! and the console mode on Windows).

#![deny(unsafe_code)]

pub mod console;
pub mod fs;
pub mod ids;
mod paths;
pub mod shell;

pub use paths::{AppPaths, PathsError, SystemPaths, linux, macos, windows};
