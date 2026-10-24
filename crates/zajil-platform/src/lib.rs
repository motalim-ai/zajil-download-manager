//! Zajil platform layer.
//!
//! Everything that differs between Windows, macOS and Linux lives behind a
//! trait in this crate, with one implementation per OS chosen at build time
//! (ADR-0006). Code outside this crate must not use `cfg(target_os)`.
//!
//! Phase M1 provides [`AppPaths`], the fixed identifiers from ADR-0001 and
//! the [`fs`] helpers for the safe-write rules of ADR-0004. Phase M3 adds
//! [`power`] (`PowerControl`).
//!
//! `unsafe` is denied everywhere except the small, commented system calls in
//! [`fs`], [`console`] and, on Windows, [`power`] that the standard library
//! does not offer (no-replace and write-through renames, hard-link counts,
//! space reservation, sparse files, free space, files only their owner may
//! read, the console mode, and shutting down or sleeping on Windows).

#![deny(unsafe_code)]

pub mod console;
pub mod fs;
pub mod ids;
pub mod ipc;
pub mod native_host;
mod paths;
pub mod power;
pub mod session;
pub mod shell;

pub use paths::{AppPaths, PathsError, SystemPaths, linux, macos, windows};
