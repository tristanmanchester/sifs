# Agent-native scorecard

This scorecard tracks SIFS against Trevin Chow's 2026 "10 Principles for
Agent-Native CLIs". The agent-facing surface is the CLI, the `sifs agent`
skill/snippet installer, the MCP server, generated agent guidance, structured
diagnostics, profiles, and the local feedback log. Search ranking internals
sit below this contract and aren't part of the agent surface.

## Score summary

| Principle | Status | Evidence |
|-----------|--------|----------|
| 1. Non-interactive by default | Complete | `--no-input` is global, commands never prompt, and bypass paths use `--force` rather than interactive confirmation. |
| 2. Structured, parseable output | Complete | Core, diagnostic, setup, cache/model, daemon, profile, feedback, and dry-run surfaces expose `--json`. JSONL is reserved for result streams. |
| 3. Errors that teach and enumerate | Complete | CLI enum parsing comes from Clap. MCP search mode and limit parsing rejects invalid values with a structured error. |
| 4. Safe retries and mutation boundaries | Complete | Cache and project clean, profile delete, init/install replacement, snippet insertion, and daemon install flows use explicit `--dry-run` and `--force` contracts. |
| 5. Bounded responses | Complete | Search and file-list payloads include `limit`, `truncated`, warnings, and narrowing hints. |
| 6. Cross-CLI vocabulary consistency | Complete | The canonical vocabulary is `source`, `filter-path`, `limit`, `list-files`, `symbol`, `outline`, `pack`, `get`, `status`, `--json`, `--force`, and `--dry-run`. |
| 7. Three-layer introspection | Complete | Human help, `sifs agent-context --json`, `sifs agent doctor`, MCP `agent_context`, MCP resources, and generated agent guidance cover all three layers. |
| 8. Async-aware execution | Partial | SIFS is synchronous. `--timeout` and `--no-input` cover the bounded-execution checks. A durable jobs ledger isn't shipped because SIFS has no long-running mutations yet. |
| 9. Persistent identity through profiles | Complete | `sifs profile` saves reusable source, search, model, and cache defaults and exposes them through `agent-context` and MCP profile tools. |
| 10. Two-way I/O | Partial | `sifs feedback` and the matching MCP tools log feedback locally. Hosted feedback delivery isn't implemented. |

## Agent-facing entities

- Source: a local path or Git URL selected with `--source`, MCP `source`, or a
  profile.
- Profile: a saved source, search, model, and cache context for repeated
  invocations.
- Index: the searchable representation of a source.
- Chunk: an indexed code or document span.
- Search request: query plus mode, limit, filters, and profile/source context.
- Search result: structured chunk match returned to CLI or MCP callers.
- Feedback entry: local JSONL record of agent friction.
- Agent artifact: a local skill package, `.claude/agents/sifs-search.md`, or a
  managed `AGENTS.md` or `CLAUDE.md` snippet.

## Discovery surfaces

- Human help: `sifs --help` and subcommand help.
- Structured context: `sifs agent-context --json`.
- Agent integration: `sifs agent print`, `sifs agent install`, and
  `sifs agent doctor --target <target> --json`.
- MCP context: `agent_context`, `sifs://agent/context`, `tools/list`, and
  `resources/list`.
- Workflow guidance: `skills/sifs-search/SKILL.md`,
  `src/agents/sifs-search.md`, and the MCP server instructions.

## What the agent contract covers

Prompt and spec material that agents read directly:

- MCP server instructions: `src/agents/mcp-instructions.md`
- MCP tool guidance: `src/agents/tools/*.md`
- MCP recovery messages: `src/agents/messages/*.md`
- Generated agent workflow: `src/agents/sifs-search.md`
- Canonical skill workflow: `skills/sifs-search/SKILL.md`
- CLI and MCP reference docs: `docs/cli.md`, `docs/mcp.md`

Engine internals that aren't part of the agent contract:

- File walking and ignored directories.
- Chunking and language detection.
- Embedding model loading.
- BM25, dense search, hybrid ranking, and reranking.
- Cache validation and JSON-RPC framing.

## Verifying the scorecard

Run these before publishing an updated scorecard:

```bash
cargo test
cargo run --bin sifs -- agent-context --json
cargo run --bin sifs -- search "agent context" --source . --mode bm25 --json
cargo run --bin sifs -- mcp doctor --source . --offline --no-cache --json
```
