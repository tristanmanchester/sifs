# Changelog

All notable changes to `sifs` are documented in this file. This file is the
source of truth for GitHub release notes, so every user-facing change belongs
under `Unreleased` before the next version is tagged.

The format is based on Keep a Changelog, and this project uses semantic
versioning where practical.

## Unreleased

### Added

- Added indexed structural inspection with `sifs symbol`, `sifs outline`,
  `list-files --prefix`, and matching MCP `symbol`, `outline`, and `pack`
  tools over existing SIFS index data.
- Added repeatable `--kind` filters for `sifs symbol` and `sifs outline`,
  matching MCP `kind`/`kinds` filters, and symbol role/confidence/origin
  metadata in structured symbol postings.
- Added `scripts/field-test-structural-tools.sh` for deterministic real-repo
  smoke testing of status, file listing, symbols, outlines, and context packs.
- Added `benchmarks/codedb_compare.py` and an example task file for repeatable
  local SIFS-vs-codedb timing and quality comparisons across search, symbol,
  and outline tasks.
- Added timeout recording to the codedb comparison benchmark so broad real-repo
  corpora can report slow setup or query cases instead of aborting the run.

### Changed

- Updated the machine-readable agent context, MCP guidance, CLI docs, README,
  and bundled SIFS skill guidance to teach symbol lookup, file outlines,
  indexed-path narrowing, and MCP context packs.
- Bumped the machine-readable agent context schema version for the structural
  tool contract expansion.
- Bounded `outline` output by default with symbol and chunk limits so large
  files remain agent-safe.
- Tightened Python and Swift symbol extraction so outlines emphasize
  declarations instead of common call-like tokens.
- Limited `pack --include-symbol-definitions` expansion to identifier-like
  query terms so broad lowercase prose queries do not pull unrelated definition
  chunks.
- Added compact structural and derived-map caches for file lists, symbol lookup,
  outlines, and warm sparse-index loads so one-shot CLI commands do less
  repeated index reconstruction when the daemon is not running.
- Split file/symbol navigation metadata into a smaller cache and use it for
  one-shot `list-files`, `symbol`, and exact-symbol BM25 search paths.
- Use the smaller navigation cache for one-shot `outline --no-chunks` when a
  file has indexed symbols, avoiding full outline-cache loads for symbol-only
  outline requests.
- Added clean-Git source fingerprints and sharded navigation manifests so
  one-shot symbol and symbol-only outline commands can avoid full file-signature
  scans and monolithic navigation-cache loads on unchanged repositories.
- Narrowed the exact-symbol search fast path to identifier-shaped BM25 queries
  and stopped scanning every indexed file for filler literal matches, keeping
  large-repo exact searches on cached symbol data before falling back to normal
  BM25 retrieval.
- Added a local single-file outline path so explicit `sifs outline <file>`
  requests can inspect that file directly instead of reconstructing repo-wide
  navigation data when no cached outline shard is usable.
- Made local `outline --no-chunks` use the lightweight line-pattern symbol
  extractor directly, avoiding tree-sitter parser startup when chunk content is
  not requested.
- Memoized Git source fingerprints within each SIFS process so short one-shot
  commands do not run duplicate Git status/rev-parse probes while validating
  manifest and shard caches.
- Prefer direct local file inspection before cache lookup for explicit
  `outline --no-chunks` requests, making the current file the fast source of
  truth for one-shot outlines.
- Added a lightweight one-token literal BM25 search path for local code
  searches, using SIFS ignore rules and definition/test-aware scoring before
  falling back to the full sparse index.
- Added a lightweight local symbol scan for one-shot symbol lookups where
  source files can be inspected faster than validating and loading navigation
  symbol shards.
- Edited README, AGENTS.md, and every file in `docs/` to remove defensive
  hedging and pedantic exclusions, and documented previously undocumented
  flags including `search --explain`, `search --context-lines`, `mcp install
  --name`, `update --update-timeout`, `profile save --ref`, `agent uninstall`,
  the `capabilities` command, and `sifs-benchmark` flags
  (`--candidate-diagnostics`, `--candidate-diagnostics-depth`,
  `--hybrid-timing`, `--no-cache`, `--alpha`).

### Fixed

- Fixed MCP `list_files` so the documented explicit `limit: 200` value is
  accepted instead of failing the shared search-result limit cap.
- Fixed MCP structural tools so `list_files`, `symbol`, `outline`, and `pack`
  reuse a running SIFS daemon before falling back to the embedded MCP index.
- Fixed `outline --json` misses so agents receive a structured `found: false`
  payload before the command exits non-zero.
- Fixed `symbol --jsonl` and `list-files --jsonl` records so each line keeps
  lookup metadata and truncation context.
- Fixed MCP structural tools so per-call document and extension indexing
  options can be supplied without relying on a saved profile.
- Fixed daemon-backed `outline` so CLI and MCP callers do not serialize chunks
  or symbols beyond the requested limits before applying output truncation.
- Fixed `sifs status` so cache, document, and extension scope flags are accepted
  consistently with other indexing commands.
- Fixed the codedb comparison benchmark so codedb `find` output contributes
  symbol hit paths and ranks instead of being counted as an empty result.
- Fixed Clippy warnings for stable Rust: type complexity, boolean expression
  simplification, manual div_ceil usage, and redundant conditional branches.

## 0.3.4 - 2026-05-26

### Changed

- Improved README first-run onboarding so new users can tell what SIFS is,
  run a fully offline first search, and choose the right next command for
  agent, semantic search, context-pack, and inspection workflows.

### Fixed

- Fixed code chunking fallback behavior so fresh machines without cached
  tree-sitter language parsers still produce non-overlapping chunks with symbol
  breadcrumbs.
- Fixed default indexing ignores so generated directories and lockfiles are
  skipped case-insensitively on Linux.
- Fixed search path filters so relative paths containing `..` parent segments
  match their normalized indexed files.
- Fixed BM25 indexing so symbol names and file stems are not double-counted in
  term frequency scoring.
- Fixed daemon startup so dangling symlinks at the socket path are reclaimed
  instead of causing bind failures.
- Fixed skill uninstall safety so directories without `SKILL.md` require
  `--force` before recursive removal.
- Fixed `sifs agent-context --json` so file inspection and related-code
  commands advertise their model, cache, and download-safety flags in the
  machine-readable contract.
- Fixed `sifs agent-context --json` so eval and tune advertise their model,
  encoder, and download-safety flags in the machine-readable contract.
- Fixed `sifs agent-context --json` so search and pack advertise their
  persistent cache-control flags in the machine-readable contract.
- Fixed `sifs agent-context --json` so search and pack advertise their
  model, encoder, and download-safety flags in the machine-readable contract.
- Fixed `sifs agent-context --json` so the advertised `eval --limit` default
  matches the CLI's actual default.
- Fixed explicit `--encoder` flags so they override saved profile encoder
  defaults for search, pack, and related-code commands.
- Fixed daemon-backed `sifs search --explain` so JSON results include ranking
  evidence when a running daemon handles the search.
- Fixed saved profile Git refs so profile-backed search, pack, file listing,
  status, get, related-code, and MCP serving use the pinned branch or tag.
- Fixed profile-backed file listing, status, get, and related-code inspection
  so saved document and extension indexing options are honored.
- Fixed tree-sitter code chunking by upgrading `tree-sitter-language-pack`
  from `1.8.0-rc.26` to `1.8.1`, fixing a broken parser manifest download URL
  that caused tree-sitter-backed chunking and symbol extraction to fall back to
  line-based chunking and restoring the new owned-`Node` API.
- Fixed `--force` uninstall help text to document that directories missing
  `SKILL.md` also require `--force`.

## 0.3.3 - 2026-05-07

### Added

- Added a Dockerfile for running the SIFS MCP server behind `mcp-proxy` in
  server listing deployments.

### Changed

- Strengthened top-rank BM25/semantic agreement and narrowed path-intent
  candidate injection in hybrid ranking, then recognized public Rust re-export
  surfaces, schema/type API queries, and implementation-to-`impls` path
  matches, boosted dotted member references such as `app.set` to their leaf
  definitions, and resolved explicit attribute queries such as `JsonProperty
  attribute` to suffixed declarations, refreshing the full benchmark baseline
  to NDCG@10 0.8471 with 2.7 ms warm uncached queries.
- Updated `sifs agent-context --json` to describe the newer search flags and
  `pack`, `eval`, and `tune` commands for agent integrations.
- Capped the in-memory per-index query-result cache with
  `SIFS_QUERY_CACHE_ENTRIES` to bound long-running daemon and MCP sessions.
- Added default document-indexing guardrails that skip common generated
  lockfiles and files larger than 1 MB.
- Normalized `filter_paths` during search so leading `./`, repeated
  separators, and Windows separators match indexed repository-relative paths.
- Added first-use semantic/search latency fields to `sifs-benchmark` output and
  documented that benchmark runs should use `--no-cache` for cold-index claims.
- Updated `sifs pack` to use saved profiles and the same mode, cache, model,
  document, and extension indexing options as search.
- Extended `sifs pack` with `--include-neighbors` and
  `--include-symbol-definitions` so context bundles can include adjacent code
  and query-named symbol definitions.
- Added file-header context to `sifs pack` output when a primary result comes
  from later in a file and the header chunk fits the remaining budget.
- Extended `sifs eval --from-feedback` with `--mode`, `--all-modes`, MRR, and
  NDCG reporting so feedback evals are no longer BM25-only.
- Expanded chunk symbol extraction for common exported, async, Rust `impl`, and
  arrow-function declaration forms.
- Documented the current Windows story and added daemon platform support details
  to `sifs doctor --json`.
- Added generic path-token candidate expansion so strong path matches can enter
  hybrid ranking even when sparse or semantic retrieval missed them initially.
- Made `sifs tune --from-feedback --dry-run` report candidate modes, alpha
  values, and follow-up eval commands instead of only counting feedback cases.
- Made `sifs tune --from-feedback --dry-run` evaluate candidate modes and
  hybrid alpha values against feedback cases for a concrete best-candidate
  report.
- Regenerated the full benchmark result artifact, generated figures, and
  benchmark documentation with reproducibility and first-use latency fields.
- Added opt-in benchmark candidate diagnostics for distinguishing candidate
  generation misses from reranking misses in per-task benchmark output.
- Stopped benchmark plot generation from reading stale mode-ablation result
  files for the query-type figure.
- Expanded hybrid candidate pools adaptively so natural-language and
  architecture-style queries retain more BM25 and semantic candidates before
  final reranking.
- Added exact symbol-definition postings as a first-class hybrid candidate
  source so query-named symbols can enter the candidate union directly.
- Added conservative token expansion for plurals, common verb forms,
  serialise/serialize spelling variants, and code-domain aliases such as
  auth/authentication and req/request.
- Embedded path, language, symbols, and breadcrumbs alongside chunk content for
  semantic vectors, with a cache version bump to avoid reusing content-only
  vectors.
- Made test, docs/example, and TypeScript declaration penalties query-aware so
  those surfaces are not down-ranked when the query explicitly asks for them.
- Changed internal search and query-cache storage to use chunk identifiers
  before materializing public `SearchResult` payloads, reducing chunk-content
  cloning in hot query paths.
- Bounded dense-search top-k candidate buffers on unfiltered queries so large
  indexes do not retain every vector score before truncation.
- Added a benchmark-discipline test that fails if production ranking/search
  code includes checked-in benchmark repository names.
- Added repo-level benchmark candidate diagnostic summaries for top-10,
  reranking, depth, and candidate-generation failure stages.
- Added a lightweight file-card candidate generator for architecture and
  natural-language hybrid queries using file paths, languages, symbols, and
  breadcrumbs as file-level retrieval metadata.
- Made benchmark candidate diagnostics compute BM25 and semantic target ranks
  from explicit per-mode diagnostic searches instead of only from final hybrid
  result explanations.
- Added a generic hybrid agreement signal so chunks found near the top by both
  BM25 and semantic retrieval are less likely to be displaced by one-sided
  ranking boosts.
- Reduced file-card candidate score weight and require path-term overlap before
  expanding into file metadata, so file-level architecture recall does not
  overpower stronger chunk-level retrieval evidence.
- Limited exact symbol candidate injection to symbol-like queries so ordinary
  prose searches are not pulled toward unrelated files that merely define a
  query word as a symbol.
- Expanded symbol extraction for common visibility/modifier prefixes, C macros,
  C typedef structs/enums, and C-like function declarations.
- Narrowed file-card candidate expansion so ordinary natural-language searches
  only use repository-wide file-card scans on smaller indexes, while explicit
  architecture, file, or location-oriented queries can still use file-level
  metadata across larger repositories.
- Replaced benchmark-specific ranking path boosts with generic path-token,
  filename, and intent signals so production scoring no longer contains
  hard-coded benchmark query and repository paths.
- Added `sifs-benchmark --no-cache` and per-repository reproducibility metadata
  so cold-index benchmark runs can be reported without persistent cache reuse.
- Added optional score explanations for CLI, daemon, and MCP search results so
  `--explain` / `explain: true` can show ranking evidence for returned chunks.
- Added symbol and breadcrumb metadata to code chunks and indexed that metadata
  in BM25 so symbol-bearing chunks are easier to search and inspect.
- Added `sifs search --include-docs`, repeatable `--extension`, and matching
  profile fields for searching documentation and config files explicitly.
- Added MCP local-index freshness checks that refresh stale cached indexes before
  search and report freshness in structured search responses.
- Added expected-query feedback fields and `sifs eval --from-feedback` for a
  local hit-rate regression loop from recorded agent misses.
- Added `sifs pack --budget-tokens` for building deduplicated, budgeted context
  bundles from a search query.
- Added `sifs tune --from-feedback --dry-run` to inspect local feedback-case
  tuning readiness without mutating ranking behavior.

### Fixed

- Fixed shallow C-like symbol extraction so ordinary call expressions such as
  `manager.validate()` are not indexed as function declarations.
- Fixed daemon-backed search so `--include-docs` and repeatable `--extension`
  use the same index filters as direct CLI search, including case-insensitive
  extension normalization.
- Fixed MCP profile searches so saved indexing options such as
  `include_docs`, `extensions`, cache settings, model, encoder, and offline
  policy are applied to local and daemon-backed indexes instead of only search
  mode and limit.
- Fixed `sifs status` semantic-cache detection to recognize current project
  semantic cache files.
- Preserved indexing warnings in sparse cache payloads and bumped the persistent
  cache version to avoid reusing older warning-less payloads.
- Fixed local index freshness checks to compare against the indexed source
  directory and indexing options instead of the persistent cache directory.
- Switched persistent index cache keys and model fingerprints from
  process-random hashers to SHA-256-derived identifiers.
- Enforced MCP search validation for `alpha`, `limit`, `filter_languages`, and
  `filter_paths` instead of only advertising those constraints in the schema.
- Skipped unreadable or non-UTF-8 files during indexing with structured
  warnings instead of aborting the entire index build.
- Restored standard triple-backtick Markdown fences for search result snippets
  whose content does not itself contain backtick fences.
- Fixed the release-check workflow and bundled Homebrew formula so tag builds
  install the current release formula from a temporary local Homebrew tap.

## 0.3.2 - 2026-05-06

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
- Fixed daemon startup checks so symlinked Unix socket paths are probed and
  stale symlinked sockets can be reclaimed.
- Fixed agent artifact rendering for `--target all --artifact mcp` so targets
  that do not support MCP are skipped instead of aborting supported output.
- Fixed Model2Vec loading to reject tokenizers whose configured unknown token
  is missing from the vocabulary instead of silently producing zero vectors.
- Fixed daemon runtime directory permissions so the unauthenticated Unix socket
  is kept inside an owner-only directory on Unix systems.
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
