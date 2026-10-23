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
- M1: single-connection HTTP/HTTPS downloads in `zajil-core`: the first response is the probe (size, `Accept-Ranges`, `ETag`, `Last-Modified`, name from `Content-Disposition` or the URL; inside quoted names only `\"` and `\\` are unescaped, so unescaped Windows paths are still stripped to their last segment); at most 10 redirects and never from https to http; rustls with the `ring` provider and the system certificate verifier; data synced before the state file counts it; atomic rename at the end; existing files are never overwritten.
- M1: tests proving that HTTPS with a self-signed certificate, a certificate for another host, or a certificate that does not cover an IP address is refused before any request is sent, plus a guard test that no source file calls a TLS-bypass API.
- M1: resume from the state file with `Range` and `If-Range` (strong `ETag`, else `Last-Modified`): a `206` must continue exactly where the data ends with the same size and validator; a `200` (changed file or ignored ranges) starts over from that response without mixing content; falls back to the original URL when the final one fails; refuses damaged state files before any request and downloads locked by another process; 64-bit offsets tested beyond 4 GiB.
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
