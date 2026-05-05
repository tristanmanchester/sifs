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
target/release/sifs init --force --json
target/release/sifs model status --json
target/release/sifs model pull --json
target/release/sifs cache status --json
target/release/sifs cache clean --dry-run --json
target/release/sifs cache clean --force --json
```

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
