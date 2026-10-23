# Changelog / سجل التغييرات

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/).

كل التغييرات المهمة تُسجَّل هنا.

## [Unreleased]

### Added
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
