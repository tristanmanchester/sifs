# MCP server usage

SIFS can run as a local Model Context Protocol server over stdio. The MCP
surface mirrors the agent-native CLI vocabulary: use `source` for local paths or
Git URLs, `limit` for bounded result counts, and `list_files` for indexed file
inventory.

MCP is optional for agent integration. The `sifs agent` command can install
CLI-first skills and snippets that tell agents to fall back to shell commands
when MCP tools are not visible in the current session.

## Start the server

```bash
target/release/sifs mcp
target/release/sifs mcp --source /path/to/project
target/release/sifs mcp --source https://github.com/owner/project --ref main
```

When a default source is configured, tools can omit `source`. Calls can still
pass another `source` when an agent needs a different local checkout or Git URL.
`--offline` rejects Git URL sources and disables model downloads.

## Install into Codex or Claude Code

```bash
sifs daemon install-agent
sifs mcp install --client all
sifs mcp install --client all --source /path/to/project
sifs mcp install --client codex --source /path/to/project
sifs mcp install --client claude --scope local --source /path/to/project
```

Use `--dry-run --json` when an agent needs the exact command arrays and fallback
config without mutating client state.

```bash
sifs mcp install --dry-run --json --client all
```

Use `sifs mcp doctor --json` for machine-readable readiness checks. The doctor
keeps MCP startup handshakes separate from BM25 search smoke.

```bash
sifs mcp doctor --source /path/to/project --offline --no-cache --json
```

## Protocol surface

`initialize` is lightweight and does not build an index. Index work happens
during tool calls such as `search`, `index_status`, and `refresh_index`.

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
- `agent_doctor`: inspect agent artifact readiness; reports `unknown` when
  current-session visibility cannot be proven.
- `search`: search chunks by natural language, code, or symbol query.
- `find_related`: find chunks related to a known file and line.
- `index_status`: inspect the selected source.
- `refresh_index`: rebuild the selected in-memory index.
- `clear_index`: remove the selected source from the in-memory MCP cache.
- `list_files`: list repository-relative indexed file paths.
- `get_chunk`: read the indexed chunk containing a file and line.
- `init_agent`: compatibility helper for writing the Claude Code SIFS agent
  file. Prefer `agent_print` plus CLI `sifs agent install` for new
  target-aware workflows.

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

Invalid modes and invalid limits are rejected instead of silently defaulting.
Structured search results include `limit`, `truncated`, warnings, index stats,
and result rows.

## Index inspection

```json
{"source": "/path/to/project"}
```

Use that shape with `index_status`, `refresh_index`, and `clear_index`.

Use `list_files` with a bounded limit:

```json
{"source": "/path/to/project", "limit": 200}
```

Use `get_chunk` with a file and one-based line:

```json
{"source": "/path/to/project", "file_path": "src/auth.rs", "line": 42}
```

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

## Breaking vocabulary

The agent-native surface intentionally uses `source` instead of `repo`, `limit`
instead of `top_k`, and `list_files` instead of `list_indexed_files`. Old names
are rejected so agents learn one contract instead of carrying ambiguous aliases.
