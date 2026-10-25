//! Zajil's batch downloads and site grabber (M7, ADR-0010).
//!
//! A UI-less library on top of `zajil-core`, like the engine: it never
//! prints, and every file is downloaded by the engine.

#![forbid(unsafe_code)]

pub mod pattern;
