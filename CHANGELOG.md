# Changelog / سجل التغييرات

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/).

كل التغييرات المهمة تُسجَّل هنا.

## [Unreleased]

### Added
- M1: `zajil-platform` crate with the `AppPaths` interface and its Windows, macOS and Linux implementations, plus the ADR-0001 identifiers as constants.
- M1: file-name sanitization (RFC 6266 §4.3, Windows reserved names and characters, hidden text-direction marks, length) and confinement to the download folder, with the same rules on every OS.
- M1: per-download state file (`.zajil-state`, schema v1, documented in `docs/STATE-FILE.md`): SHA-256 checksum, strict reading that rejects damaged files, and atomic writes with a folder sync (`zajil_platform::fs::sync_dir`).
- M1: `zajil-testserver`, a local HTTP/HTTPS test server for hard cases: no range support, cut connections, changed files, slow and stalled transfers, 429 with `Retry-After`, 403, redirects, dangerous `Content-Disposition` names, and a request log.
- M0 foundation:
  - README, PRD, architecture, roadmap (M1–M8) and IDM feature-parity table.
  - ADRs 0001–0006.
  - Contributing guide with the clean-room rule, security policy and code of conduct.
  - Issue and PR templates.
  - Empty Rust workspace.
  - CI on Windows, macOS and Linux: fmt, clippy, test and license check.
  - All CI actions pinned to commit SHAs (Dependabot updates them).
  - Rust toolchain pinned in `rust-toolchain.toml`.
  - `AGENTS.md`: work rules for AI agents.
- Owner decisions recorded:
  - ADR-0003, ADR-0004 and ADR-0005 accepted.
  - License set to GPL-3.0-or-later.
  - App ids:
    - `io.github.motalim-ai.zajil` for macOS and Tauri.
    - `io.github.motalim_ai.zajil` for Linux and the Native Messaging host (`.host`).
  - FTP moved to 1.x.
