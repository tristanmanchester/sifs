# Changelog

All notable changes to `sifs` are documented in this file. This file is the
source of truth for GitHub release notes, so every user-facing change belongs
under `Unreleased` before the next version is tagged.

The format is based on Keep a Changelog, and this project uses semantic
versioning where practical.

## Unreleased

### Added

- Added `sifs agent-context --json` and MCP `agent_context` discovery so
  agents can inspect the CLI/MCP contract without scraping help text.
- Added persistent `sifs profile` commands for saved source/search/model/cache
  defaults.
- Added local-first `sifs feedback` commands and MCP feedback tools for agent
  friction reports.

### Changed

- Redesigned the greenfield CLI/MCP vocabulary around explicit agent-native
  names: `--source`, `--filter-path`, `--limit`, `list-files`, MCP `source`,
  MCP `limit`, and MCP `list_files`.
- Added JSON output to diagnostic, setup, install dry-run, daemon, model, cache,
  init, capabilities, profile, and feedback surfaces.
- Added bounded-output metadata such as `limit`, `truncated`, and narrowing
  hints to search and file-list payloads.
- Updated CLI, MCP, generated agent guidance, and the agent-native scorecard for
  the new Trevin Chow 10-principle contract.
- Added a plan for a breaking agent-native CLI/MCP redesign covering
  `agent-context`, canonical source/filter vocabulary, uniform JSON diagnostics,
  strict validation, profiles, feedback, and contract-level tests.
- Refreshed benchmark result artifacts and comparison graphs from the latest
  full benchmark run.
- Tuned natural-language ranking to use file-stem and parent-directory matches
  when boosting path-relevant chunks.
- Recognized TypeScript-style `$`-prefixed internal symbol definitions when
  ranking bare symbol searches.
- Added narrow subsystem path-intent boosts for queries that explicitly name
  public surfaces, worker setup, update state, UI views, or deserialization
  entry points.
- Added a diagnostic benchmark timing flag for breaking down hybrid query time.

### Performance

- Avoided allocating an all-chunk candidate list for unfiltered dense searches.
- Combined hybrid RRF scores directly instead of materializing intermediate
  score maps.
- Reused file-to-chunk mappings when applying exact path-intent boosts.

### Fixed

- Rejected invalid MCP search modes and invalid limits instead of silently
  defaulting to hybrid search.
- Rejected stale MCP `repo` arguments and empty MCP `source` arguments so tool
  calls no longer silently index the server working directory instead of the
  intended source.

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
