//! Zajil platform layer.
//!
//! Everything that differs between Windows, macOS and Linux lives behind a
//! trait in this crate, with one implementation per OS chosen at build time
//! (ADR-0006). Code outside this crate must not use `cfg(target_os)`.
//!
//! Phase M1 provides [`AppPaths`], the fixed identifiers from ADR-0001 and
//! the [`fs`] helpers for the safe-write rules of ADR-0004. Phase M3 adds
//! [`power`] (`PowerControl`); M5 [`clipboard`], [`ipc`] and
//! [`native_host`].
//!
//! `unsafe` is denied everywhere except the small, commented system calls in
//! [`fs`], [`console`], [`process`] and, on Windows, [`power`], [`session`], [`ipc`],
//! [`native_host`] and [`clipboard`] that the standard library does not
//! offer (no-replace and write-through renames, hard-link counts, space
//! reservation, sparse files, free space, files only their owner may read,
//! the console mode, shutting down or sleeping, the end of the session,
//! named pipes, the registry and the clipboard on Windows, and the process
//! groups, job objects and death signal that end a tool's process tree).

#![deny(unsafe_code)]

pub mod clipboard;
pub mod console;
pub mod fs;
pub mod ids;
pub mod ipc;
pub mod native_host;
pub mod origin;
mod paths;
pub mod power;
pub mod process;
pub mod session;
pub mod shell;
pub mod target;

pub use paths::{AppPaths, PathsError, SystemPaths, linux, macos, windows};
