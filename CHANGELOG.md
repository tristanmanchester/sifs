# Changelog

All notable changes to `sifs` are documented in this file. This file is the
source of truth for GitHub release notes, so every user-facing change belongs
under `Unreleased` before the next version is tagged.

The format is based on Keep a Changelog, and this project uses semantic
versioning where practical.

## Unreleased

### Fixed

- Fixed MCP search and related-code tools so saved profile mode and limit
  defaults apply when tool calls omit explicit values.
- Fixed identifier tokenization so names with leading or trailing underscores
  remain searchable by their inner token.
- Fixed profile saves to use a temporary file and rename so a failed write does
  not truncate existing `profiles.json` data.
- Fixed syntax-aware chunking so oversized leaf nodes such as long string
  literals are split instead of producing oversized search chunks.
- Fixed daemon client timeout handling so `--timeout 0` is treated explicitly
  and nonzero socket timeout configuration errors are reported.
- Fixed human-readable CLI and MCP code blocks so matched content containing
  triple backticks no longer breaks markdown rendering.
- Fixed `sifs cache clean --force` so the human-readable output no longer
  claims a missing cache directory was removed.
- Fixed GitHub Actions CI by aligning the workflow MSRV with current parser
  dependencies, keeping ClawHub workflow Cargo commands locked, and making
  update-command tests independent of runner `CARGO_HOME`.

## 0.3.1 - 2026-05-05

### Added

- Added GitHub Actions CI, release-check, and benchmark workflows for Rust
  formatting, linting, tests, packaging checks, MSRV coverage, and manual
  diagnostic benchmark runs.
- Added an implementation plan for a package-manager-backed `sifs update`
  command, with passive update-available notices deferred as follow-up work.
- Added `sifs update` with check, dry-run, JSON, Cargo/Homebrew ownership gates,
  and package-manager-backed mutation for safely updating installed binaries.

### Changed

- Added explicit Homebrew and Cargo installation guidance to the `sifs-search`
  skill troubleshooting docs and setup-check scripts for agents that do not
  already have `sifs` on `PATH`.

### Fixed

- Reclaimed stale daemon Unix sockets automatically when starting `sifs daemon run`
  without requiring `--replace-existing-socket`.

## 0.3.0 - 2026-05-05

### Added

- Added an ideation artifact for making SIFS more agent-native across Codex,
  Claude Code, OpenClaw, Hermes, agent skills, plugins, and MCP integrations.
- Added a CLI-first agent skill installer plan covering target-aware skill
  exports, managed `AGENTS.md`/`CLAUDE.md` snippets, and MCP-as-optional
  readiness checks.
- Added `sifs agent print`, `sifs agent install`, `sifs agent doctor`, and
  `sifs agent uninstall` for CLI-first agent skills, snippets, and readiness
  checks across Codex, Claude Code, OpenClaw, Hermes, and generic targets.
- Added a canonical `sifs-search` skill package with command, MCP, and
  troubleshooting references for agent-skill consumers.
- Added ClawHub publishing prep for the `sifs-search` skill, including
  trigger-eval fixtures, standalone OpenClaw package references, a parity test,
  a readiness/publish helper, and a manual GitHub Actions workflow.
- Added read-only MCP `agent_print` and `agent_doctor` tools so MCP clients can
  inspect agent artifacts without broad filesystem mutation.
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
- Positioned MCP as an optional agent capability and documented CLI fallback
  behavior for generated skills and snippets.
- Improved `sifs-search` skill descriptions and metadata across canonical,
  OpenClaw, Hermes, and generic agent-skill packages using trigger-oriented
  agent skill guidance.
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
