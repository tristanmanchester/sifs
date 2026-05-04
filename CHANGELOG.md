# Changelog

All notable changes to `sifs` are documented in this file. This file is the
source of truth for GitHub release notes, so every user-facing change belongs
under `Unreleased` before the next version is tagged.

The format is based on Keep a Changelog, and this project uses semantic
versioning where practical.

## Unreleased

### Fixed

- Rejected empty MCP `repo` arguments so tool calls no longer silently index the
  server working directory instead of the intended source.

## 0.2.1 - 2026-05-04

### Added

- Added an MCP hardening plan for stdio compatibility, Codex startup
  diagnostics, and timeout troubleshooting.
- Added MCP doctor handshake probes for newline-delimited and `Content-Length`
  stdio framing so startup failures are reported separately from BM25 search
  smoke.

### Fixed

- Made the MCP stdio server accept newline-delimited JSON-RPC messages while
  preserving existing `Content-Length` framing compatibility.

## 0.2.0 - 2026-05-04

### Added

- Added `sifs --version` and `sifs -V` so installed binaries can report their
  package version.
- Added a shared `sifs daemon` with Unix-socket IPC, daemon status/ping
  commands, macOS LaunchAgent install/uninstall commands, and opportunistic CLI
  reuse for warm search/index operations.
- Added daemon-first MCP installation so `sifs mcp install` can configure Codex
  and Claude without pinning the server to one source directory.

### Changed

- MCP servers started without an explicit source now default to the server
  process working directory, while tool calls can still pass an explicit `repo`
  for local paths or Git URLs.

## 0.1.1 - 2026-05-04

### Added

- Added a changelog and local agent instructions so release notes are maintained
  as changes are built.

### Fixed

- Allowed SIFS MCP tool calls to search an explicit `repo` even when the server
  was started with a default source, so one configured MCP server can search
  other local checkouts or Git URLs instead of rejecting repo overrides.
