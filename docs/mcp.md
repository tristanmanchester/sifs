# MCP server usage

SIFS runs as a local Model Context Protocol server over stdio. The MCP surface
mirrors the CLI vocabulary: `source` for local paths or Git URLs, `limit` for
bounded result counts, and `list_files` for indexed file inventory.

MCP is optional for agent integration. The `sifs agent` command installs
CLI-first skills and snippets that fall back to shell commands when MCP tools
aren't visible in the current session.

## Start the server

```bash
target/release/sifs mcp
target/release/sifs mcp --source /path/to/project
target/release/sifs mcp --source https://github.com/owner/project --ref main
```

When a default source is configured, tools can omit `source`. Calls can still
pass another `source` to target a different local checkout or Git URL.
`--offline` rejects Git URL sources and disables model downloads.

## Install into Codex or Claude Code

```bash
sifs daemon install-agent
sifs mcp install --client all
sifs mcp install --client all --source /path/to/project
sifs mcp install --client codex --source /path/to/project
sifs mcp install --client claude --scope local --source /path/to/project
```

Use `--dry-run --json` to get the exact command arrays and fallback config
without changing client state.

```bash
sifs mcp install --dry-run --json --client all
```

Use `sifs mcp doctor --json` for machine-readable readiness checks. The doctor
reports MCP startup handshake state and BM25 search smoke as separate signals.

```bash
sifs mcp doctor --source /path/to/project --offline --no-cache --json
```

## Protocol surface

`initialize` is lightweight; it doesn't build an index. Index work happens
inside tool calls such as `search`, `index_status`, and `refresh_index`.

Supported methods:

- `initialize`
- `resources/list`
- `resources/read`
- `tools/list`
- `tools/call`

Supported resources:

- `sifs://server/context`
- `sifs://agent/context`
- `sifs://profiles`
- `sifs://feedback`
- `sifs://index/status`
- `sifs://index/files`

## Tools

Core tools:

- `agent_context`: return the versioned CLI/MCP contract.
- `agent_print`: render a SIFS skill, snippet, or MCP guidance artifact without
  writing files.
- `agent_doctor`: inspect agent artifact readiness. Reports `unknown` for
  signals that depend on current-session visibility.
- `search`: search chunks by natural language, code, or symbol query.
- `find_related`: find chunks related to a known file and line.
- `index_status`: inspect the selected source.
- `refresh_index`: rebuild the selected in-memory index.
- `clear_index`: remove the selected source from the in-memory MCP cache.
- `list_files`: list repository-relative indexed file paths.
- `get_chunk`: read the indexed chunk containing a file and line.
- `symbol`: look up indexed symbols by exact or folded name.
- `outline`: inspect symbols, breadcrumbs, and chunk line spans for one indexed
  file.
- `pack`: build a bounded context pack for a task query.
- `init_agent`: compatibility helper that writes the Claude Code SIFS agent
  file. For new target-aware workflows, use `agent_print` plus CLI `sifs
  agent install`.

Profile and feedback tools:

- `profile_list`
- `profile_show`
- `feedback_create`
- `feedback_list`

## Search tool

Input schema:

```json
{
  "query": "Natural language or code query.",
  "source": "/path/to/project",
  "profile": "sifs-dev",
  "mode": "hybrid",
  "limit": 5,
  "filter_languages": ["rust"],
  "filter_paths": ["src/main.rs"]
}
```

Fields:

- `query` is required.
- `source` is optional when the server has a default source or `profile`
  provides one.
- `profile` is optional and uses saved source/search defaults.
- `mode` is `hybrid`, `semantic`, or `bm25`; defaults to the profile's mode if set, otherwise `hybrid`.
- `limit` defaults to the profile's limit if set, otherwise `5`.
- `alpha` optionally controls hybrid semantic weight.
- `filter_languages` and `filter_paths` narrow the indexed chunks searched.

Invalid modes and invalid limits are rejected with a structured error rather
than silently defaulting. Search results include `limit`, `truncated`,
warnings, index stats, and result rows.

## Index inspection

```json
{"source": "/path/to/project"}
```

Use that shape with `index_status`, `refresh_index`, and `clear_index`.

Use `list_files` with a bounded limit:

```json
{"source": "/path/to/project", "prefix": "src/auth/", "limit": 200}
```

Pass `include_docs: true` and `extensions: ["md", "json"]` on `list_files`,
`symbol`, `outline`, or `pack` when the agent needs a one-off document or
custom-extension index scope without saving a profile.

Use `get_chunk` with a file and one-based line:

```json
{"source": "/path/to/project", "file_path": "src/auth.rs", "line": 42}
```

Use `symbol` when an agent already knows the symbol name:

```json
{"source": "/path/to/project", "name": "SessionToken", "kind": "struct", "limit": 20}
```

Use `outline` to inspect one indexed file before reading raw chunks:

```json
{"source": "/path/to/project", "file_path": "src/auth/session.rs", "kinds": ["function"], "symbols_limit": 200, "chunks_limit": 100}
```

Use `no_chunks: true` for a symbol-only outline. Missing indexed paths return a
structured `found: false` response. `symbol` and `outline` accept `kind` as a
string or array, and `kinds` as an array, to filter the returned symbol kinds.

Use `pack` to retrieve task-shaped context through MCP:

```json
{
  "source": "/path/to/project",
  "query": "how session validation works",
  "mode": "hybrid",
  "budget_tokens": 6000,
  "include_neighbors": 1,
  "include_symbol_definitions": true
}
```

`include_symbol_definitions` expands only identifier-like query terms such as
`SessionToken`, `auth_flow`, or `token2`. Lowercase prose terms stay as
ordinary search context.

When the SIFS daemon is running, MCP `search`, `list_files`, `symbol`,
`outline`, and `pack` reuse its warm index and fall back to the embedded MCP
index when the daemon is unavailable. Filtered `symbol` and `outline` calls use
the embedded index so filtering applies before result limiting.

## Profiles and feedback

Profiles are saved through the CLI and discoverable through MCP:

```json
{}
```

Call `profile_list` to enumerate profiles, then `profile_show` with a name.

Feedback is local-first:

```json
{"message": "mode=lexical should enumerate valid modes"}
```

Call `feedback_create` to append a local feedback entry and `feedback_list` to
inspect bounded recent entries.

## Vocabulary

The MCP surface uses `source` (not `repo`), `limit` (not `top_k`), and
`list_files` (not `list_indexed_files`). The older names are rejected, so
agents always see one canonical contract.
