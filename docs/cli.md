# Command-line usage

SIFS is an agent-native code search CLI. The public command surface uses
explicit source vocabulary, uniform `--json` diagnostics, strict validation, and
bounded defaults so agents do not have to scrape help text or infer state.

Build the release binary before running examples from this repository.

```bash
cargo build --release
```

## Agent context

`agent-context` is the machine-readable entrypoint for agents. It describes the
CLI version, schema version, commands, flags, enum values, mutation boundaries,
MCP tools, profiles, and feedback support.

```bash
target/release/sifs agent-context --json
```

This command is intentionally cheap: it does not index sources, load models, or
touch the network.

## Agent artifacts

`agent` prints, installs, inspects, and removes target-specific SIFS integration
artifacts. Generated artifacts are CLI-first and MCP-optional.

```bash
target/release/sifs agent print --target codex --artifact snippet
target/release/sifs agent print --target generic --artifact skill --json
target/release/sifs agent install --target codex --artifact snippet --file AGENTS.md --dry-run --json
target/release/sifs agent install --target codex --artifact snippet --file AGENTS.md
target/release/sifs agent doctor --target codex --json
target/release/sifs agent uninstall --target codex --artifact snippet --file AGENTS.md --dry-run --json
```

Targets are `codex`, `claude-code`, `openclaw`, `hermes`, `generic`, and
`all`. Artifacts are `skill`, `snippet`, `mcp`, and `all`.

Snippet installs write only a SIFS managed block:

```markdown
<!-- BEGIN SIFS AGENT INSTRUCTIONS schema=1 checksum=... -->
...
<!-- END SIFS AGENT INSTRUCTIONS -->
```

Re-running an identical install is a no-op. User-modified managed blocks require
`--force`. See [agent-integration.md](agent-integration.md) for target details.

## Search

`search` indexes the selected source and returns ranked chunks. `--source` is
the only canonical way to name a local directory or Git URL. `--filter-path` is
only for repository-relative indexed file filters.

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

Mode values are:

- `hybrid`: combine semantic and BM25 rankings with query-aware reranking.
- `semantic`: rank by embedding similarity only.
- `bm25`: rank by sparse lexical matching only. This mode is model-free and
  never loads tokenizers, safetensors, or Hugging Face model files.

Use `--offline` to prevent all network access by SIFS, including remote Git
clones and model downloads. Use `--no-download` to prevent model downloads while
still allowing local path indexing and remote Git sources.

Structured search JSON includes `query`, `mode`, `source`, `limit`, filters,
index stats, elapsed time, warnings, `truncated`, a narrowing hint when useful,
and result objects with file path, line range, language, score, source mode, and
content. Use `--jsonl` when one JSON record per result is easier to stream.

## Context packs

`pack` builds a budgeted JSON context bundle for agents. It uses the same
source/profile/mode/model/cache/document/extension resolution as `search`, then
deduplicates primary ranked chunks by file. Use `--include-neighbors` to include
adjacent chunks around selected results, and `--include-symbol-definitions` to
add chunks that define symbols named in the query when they fit the remaining
budget.

```bash
target/release/sifs pack "how request auth works" \
  --source . \
  --mode hybrid \
  --budget-tokens 6000 \
  --include-neighbors 1 \
  --include-symbol-definitions \
  --json
```

Pack items include `kind`, file path, line range, optional score, source mode,
symbols, breadcrumbs, a short inclusion reason, and content.

## Feedback eval and tuning

`eval --from-feedback` measures local feedback cases against one mode or all
modes. `tune --from-feedback --dry-run` uses the same feedback cases to score
candidate modes and hybrid alpha values without mutating ranking defaults.

```bash
target/release/sifs eval --from-feedback --source . --all-modes --json
target/release/sifs tune --from-feedback --source . --dry-run --json
```

## Related code

`find-related` starts from a known repository-relative file path and one-based
line number, then finds similar chunks in the same source.

```bash
target/release/sifs find-related src/auth/session.rs 42 --source /path/to/project --limit 8 --json
```

## Index inspection

Use `list-files`, `status`, and `get` to inspect what SIFS indexed.

```bash
target/release/sifs list-files --source /path/to/project --limit 200 --json
target/release/sifs status --source /path/to/project --json
target/release/sifs get src/auth/session.rs 42 --source /path/to/project --json
```

`list-files --json` includes `total`, `limit`, `truncated`, and a hint when the
file list is incomplete.

When the shared daemon is running, `search`, `find-related`, `list-files`,
`status`, and `get` opportunistically reuse warm indexes. If the daemon socket is
not available, commands fall back to direct one-shot indexing.

## Profiles

Profiles persist source and search defaults for repeated agent sessions. They
are stored locally under the SIFS platform cache root and are discoverable
through `agent-context`.

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

Precedence is explicit flag, environment, profile, then default.

## Feedback

Agents can record local feedback when SIFS causes friction. Feedback is
append-only JSONL stored in the SIFS platform cache root.

```bash
target/release/sifs feedback create "invalid mode error should list valid values" --json
target/release/sifs feedback list --limit 20 --json
```

Feedback does not send anything upstream by default.

## Diagnostics and setup

Diagnostic and setup commands support `--json` so agents can parse readiness and
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

`sifs init` remains as a compatibility shortcut for the Claude Code agent file.
Prefer `sifs agent` for new target-aware skill and snippet workflows.

## Updating SIFS

`update` checks for and installs newer SIFS releases through the package manager
that owns the current binary. Cargo installs compare against crates.io.
Homebrew installs use Homebrew's manager-available formula version as the
actionable version and may report crates.io as upstream context.

```bash
target/release/sifs update --check
target/release/sifs update --check --json
target/release/sifs update --dry-run --json
target/release/sifs update --json
```

`--check` reports availability and blocking conditions without requiring a safe
mutation plan. `--dry-run` validates ownership and prints the package-manager
commands that would run. Default mutation refuses development, copied,
ambiguous, PATH-shadowed, or ownership-mismatched binaries instead of updating a
different install root.

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

On macOS, `daemon install-agent` writes a user LaunchAgent that runs
`sifs daemon run --replace-existing-socket` at login and keeps it alive.
Daemon IPC currently uses same-user Unix sockets, so daemon mode is supported on
Unix platforms only. Windows users should use direct CLI commands or MCP stdio
until a named-pipe or TCP-loopback daemon transport exists.

## MCP

Run `sifs mcp` to start stdio server mode. Use `--source` to pin the server to a
default local directory or Git URL.

```bash
target/release/sifs mcp
target/release/sifs mcp --source /path/to/project
target/release/sifs mcp --source https://github.com/owner/project --ref main
```

Install into Codex or Claude Code:

```bash
target/release/sifs mcp install --client all --dry-run --json
target/release/sifs mcp install --client codex --source /path/to/project --force
target/release/sifs mcp doctor --source /path/to/project --offline --no-cache --json
```

MCP tool calls use `source`, not `repo`, and `limit`, not `top_k`.

## Output modes

- `--json`: one structured JSON payload on stdout.
- `--jsonl`: newline-delimited result records for search-style commands.
- `--format compact`: concise human text where supported.

Errors use non-zero exit codes. Invalid enum-like values and numeric bounds are
reported early with actionable messages so agents can retry correctly.
