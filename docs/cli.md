# Command-line usage

SIFS provides a code-search CLI for agents and developers. The command surface
uses consistent `--source` vocabulary, uniform `--json` diagnostics, strict
input validation, and bounded defaults.

Build the release binary before running examples from this repository.

```bash
cargo build --release
```

Two flags apply to every command:

- `--no-input` asserts non-interactive execution. SIFS never prompts and closes
  child stdin where applicable.
- `--timeout <seconds>` bounds daemon, MCP probe, and external client
  operations. Defaults to 30.

## Agent context

`agent-context` is the machine-readable entrypoint for agents. It describes the
CLI version, schema version, commands, flags, enum values, mutation boundaries,
MCP tools, profiles, and feedback support.

```bash
target/release/sifs agent-context --json
```

The command doesn't index sources, load models, or touch the network, so it
runs in milliseconds.

## Capabilities

`capabilities` prints a compact summary of agent, CLI, and MCP capabilities.
Use it when an agent or human needs a quick readiness snapshot.

```bash
target/release/sifs capabilities --json
```

## Agent artifacts

`agent` renders, installs, inspects, and removes integration artifacts for
specific agent targets. Generated artifacts are CLI-first; MCP is optional.

```bash
target/release/sifs agent print --target codex --artifact snippet
target/release/sifs agent print --target generic --artifact skill --json
target/release/sifs agent install --target codex --artifact snippet --file AGENTS.md --dry-run --json
target/release/sifs agent install --target codex --artifact snippet --file AGENTS.md
target/release/sifs agent doctor --target codex --json
target/release/sifs agent uninstall --target codex --artifact snippet --file AGENTS.md --dry-run --json
```

Targets: `codex`, `claude-code`, `openclaw`, `hermes`, `generic`, `all`.
Artifacts: `skill`, `snippet`, `mcp`, `all`.

Snippet installs write only the managed block:

```markdown
<!-- BEGIN SIFS AGENT INSTRUCTIONS schema=1 checksum=... -->
...
<!-- END SIFS AGENT INSTRUCTIONS -->
```

Re-running an identical install is a no-op. User-modified managed blocks and
skill directories that lack `SKILL.md` require `--force` to overwrite or
remove.

`agent uninstall` removes managed skill files or instruction blocks. Use
`--dry-run` to preview removals and `--force` to remove stale, user-modified,
or unverified artifacts.

See [agent-integration.md](agent-integration.md) for full target details.

## Search

`search` indexes the selected source and returns ranked chunks. Use `--source`
to name a local directory or Git URL. Use `--filter-path` to restrict to
repository-relative file paths in the index.

```bash
target/release/sifs search <query> \
  --source <path-or-git-url> \
  --mode hybrid \
  --limit 5 \
  --language rust \
  --filter-path src/auth.rs \
  --json
```

Examples:

```bash
target/release/sifs search "where is the login redirect handled" --source .
target/release/sifs search "SessionToken" --source /path/to/project --mode bm25 --limit 10
target/release/sifs search "stream upload backpressure" --source https://github.com/owner/project
target/release/sifs search "token validation" --source . --mode bm25 --language rust --filter-path src/auth.rs --json
```

Modes:

- `hybrid`: combines semantic and BM25 rankings with query-aware reranking.
- `semantic`: ranks by embedding similarity.
- `bm25`: ranks by sparse lexical matching. Loads no model files.

Network flags:

- `--offline` blocks remote Git clones and model downloads.
- `--no-download` blocks model downloads but allows remote Git sources.

Output and detail flags:

- `--json` prints one structured payload.
- `--jsonl` prints one JSON record per result for streaming.
- `--format compact` switches the human format to a tighter single-line layout.
- `--explain` adds query parsing, ranking weights, and filter metadata to the
  output.
- `--context-lines <n>` reads up to `n` lines before and after each chunk from
  the local source file. The flag has no effect on Git URL sources or when the
  local file isn't readable.

Structured search JSON includes `query`, `mode`, `source`, `limit`, filters,
index stats, elapsed time, warnings, `truncated`, a narrowing hint when
relevant, and result objects with file path, line range, language, score,
source mode, and content.

## Context packs

`pack` builds a budgeted JSON context bundle. It uses the same source, profile,
mode, model, cache, document, and extension resolution as `search`, then
deduplicates ranked chunks by file.

Optional expansions:

- `--include-neighbors <n>` adds up to `n` adjacent chunks before and after
  each selected result.
- `--include-symbol-definitions` adds chunks that define identifier-like
  symbols named in the query, when they fit the remaining budget. Lowercase
  prose terms don't trigger this expansion.

```bash
target/release/sifs pack "how request auth works" \
  --source . \
  --mode hybrid \
  --budget-tokens 6000 \
  --include-neighbors 1 \
  --include-symbol-definitions \
  --json
```

The default budget is 6000 tokens. Pack items carry `kind`, file path, line
range, optional score, source mode, symbols, breadcrumbs, an inclusion reason,
and content.

## Feedback eval and tuning

`eval --from-feedback` measures local feedback cases against one mode or all
modes. `tune --from-feedback --dry-run` scores candidate modes and hybrid alpha
values from the same feedback cases without changing ranking defaults.

```bash
target/release/sifs eval --from-feedback --source . --all-modes --json
target/release/sifs tune --from-feedback --source . --dry-run --json
```

Both commands accept `--limit`, `--mode`, `--model`, `--encoder`, `--offline`,
and `--no-download` like `search`. `tune --dry-run` is currently the only
supported form; running without `--dry-run` is reserved for a future write path.

## Related code

`find-related` starts from a known repository-relative file path and one-based
line number, then finds similar chunks in the same source.

```bash
target/release/sifs find-related src/auth/session.rs 42 --source /path/to/project --limit 8 --json
```

## Structural inspection

Use `list-files`, `symbol`, `outline`, `status`, and `get` to inspect the
indexed surface before broad file reads. These commands operate on indexed
data and can run model-free with `--offline --no-cache`.

```bash
target/release/sifs list-files --source /path/to/project --limit 200 --json
target/release/sifs list-files --source /path/to/project --prefix src/auth/ --json
target/release/sifs symbol SessionToken --source /path/to/project --kind struct --json
target/release/sifs outline src/auth/session.rs --source /path/to/project \
  --kind function --symbols-limit 200 --chunks-limit 100 --json
target/release/sifs status --source /path/to/project --no-cache --json
target/release/sifs get src/auth/session.rs 42 --source /path/to/project --json
```

Output details:

- `list-files --json` returns `total`, `limit`, `truncated`, and a narrowing
  hint when the file list is incomplete.
- `symbol --json` returns symbol postings with repository-relative path, line,
  kind, role, confidence, origin, chunk line range, language, and breadcrumbs.
- `symbol --jsonl` emits one envelope per posting with lookup metadata and
  truncation state.
- `outline --json` returns `found`, post-filter symbol and chunk counts,
  truncation metadata, file language, line span, bounded symbols, breadcrumbs,
  and bounded chunk boundaries. Use `--no-chunks` for a symbol-only outline.

Repeat `symbol --kind` or `outline --kind` to restrict to specific symbol kinds
like `function`, `struct`, `class`, or `case`. `status`, `list-files`,
`symbol`, and `outline` share the cache, document, and extension flags.

When the SIFS daemon is running, `search`, `find-related`, `list-files`,
`symbol`, `outline`, `status`, and `get` reuse warm indexes. When the daemon
socket isn't available, they fall back to direct one-shot indexing. `outline
--json` returns a structured `found: false` payload for non-indexed paths
before exiting non-zero.

## Profiles

Profiles persist source and search defaults for repeated agent sessions. They
live under the SIFS platform cache root and are discoverable through
`agent-context`.

```bash
target/release/sifs profile save sifs-dev \
  --source . \
  --mode bm25 \
  --limit 10 \
  --offline \
  --json

target/release/sifs profile list --json
target/release/sifs profile show sifs-dev --json
target/release/sifs search "mcp handshake" --profile sifs-dev --json
target/release/sifs profile delete sifs-dev --force --json
```

`profile save` accepts every search-shaping flag (`--mode`, `--limit`,
`--encoder`, `--model`, `--offline`, `--no-download`, `--cache-dir`,
`--no-cache`, `--project-cache`, `--include-docs`, `--extension`) plus `--ref`
for pinning a Git branch or tag. `profile delete` requires `--force` to remove
a saved profile.

Precedence is explicit flag, environment, profile, then default.

## Feedback

Record local feedback when SIFS causes friction. Feedback is an append-only
JSONL log under the SIFS platform cache root and stays on the local machine.

```bash
target/release/sifs feedback create "invalid mode error should list valid values" --json
target/release/sifs feedback list --limit 20 --json
```

## Diagnostics and setup

Diagnostic and setup commands accept `--json` so agents can parse readiness and
mutation results.

```bash
target/release/sifs doctor --source . --offline --json
target/release/sifs capabilities --json
target/release/sifs agent doctor --target codex --json
target/release/sifs agent install --target codex --artifact snippet --file AGENTS.md --dry-run --json
target/release/sifs init --force --json
target/release/sifs model status --json
target/release/sifs model pull --json
target/release/sifs cache status --json
target/release/sifs cache clean --dry-run --json
target/release/sifs cache clean --force --json
target/release/sifs update --check --json
target/release/sifs update --dry-run --json
```

`sifs init` is a compatibility shortcut that writes `.claude/agents/sifs-search.md`.
For new target-aware workflows, use `sifs agent` instead.

## Updating SIFS

`update` checks for and installs newer SIFS releases through the package
manager that owns the current binary. Cargo installs compare against
crates.io. Homebrew installs use the tap formula version as the actionable
version and report crates.io as upstream context.

```bash
target/release/sifs update --check
target/release/sifs update --check --json
target/release/sifs update --dry-run --json
target/release/sifs update --json
```

Behavior:

- `--check` reports availability and any blocking conditions.
- `--dry-run` validates ownership and prints the package-manager commands that
  would run.
- The default mutation path refuses development, copied, ambiguous,
  PATH-shadowed, or ownership-mismatched binaries. It prints next actions
  rather than touching a different install root.
- `--update-timeout <seconds>` bounds the package-manager update step.
  Defaults to 600.

Project-local cache cleanup is explicit and force-gated:

```bash
target/release/sifs clean --source . --dry-run --json
target/release/sifs clean --source . --force --json
```

## Daemon

The `daemon` command manages the shared local SIFS daemon used by agent
integrations and warm CLI searches.

```bash
target/release/sifs daemon run --replace-existing-socket
target/release/sifs daemon ping --json
target/release/sifs daemon status --json
target/release/sifs daemon install-agent --dry-run --json
target/release/sifs daemon install-agent --force --json
target/release/sifs daemon uninstall-agent --dry-run --json
```

On macOS, `daemon install-agent` writes a user LaunchAgent that runs `sifs
daemon run --replace-existing-socket` at login and keeps it alive. Daemon IPC
uses same-user Unix sockets, so daemon mode runs on Unix only. On Windows, use
direct CLI commands or MCP stdio.

## MCP

Run `sifs mcp` to start stdio server mode. Use `--source` to pin the server to
a default local directory or Git URL.

```bash
target/release/sifs mcp
target/release/sifs mcp --source /path/to/project
target/release/sifs mcp --source https://github.com/owner/project --ref main
```

Install into Codex or Claude Code:

```bash
target/release/sifs mcp install --client all --dry-run --json
target/release/sifs mcp install --client codex --source /path/to/project --force
target/release/sifs mcp install --client claude --scope local --source /path/to/project
target/release/sifs mcp doctor --source /path/to/project --offline --no-cache --json
```

`mcp install` flags:

- `--client codex|claude|all` selects the target client.
- `--scope local|project|user` chooses the Claude Code scope.
- `--source` and `--ref` set the default source for the registered server.
- `--name <name>` overrides the default server name (`sifs`).
- `--force` replaces an existing server with the same name.
- `--dry-run` prints the resulting commands and config without changing client
  state.

MCP tool calls use `source` (not `repo`) and `limit` (not `top_k`).

## Output modes

- `--json` prints one structured payload on stdout.
- `--jsonl` prints newline-delimited result records for search-style commands.
- `--format compact` produces concise human text where the command supports it.

Errors use non-zero exit codes. Invalid enum values and out-of-range numbers
are reported early with actionable messages.
