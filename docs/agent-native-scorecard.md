# Agent-native scorecard

This scorecard evaluates SIFS against Trevin Chow's 2026 "10 Principles for
Agent-Native CLIs". SIFS' agent-facing surface is the CLI, MCP server,
generated agent guidance, structured diagnostics, profiles, and local feedback
log. Search ranking internals remain deterministic code-defined engine details.

## Score summary

| Principle | Status | Evidence |
|-----------|--------|----------|
| 1. Non-interactive by default | Complete | `--no-input` is global, commands do not prompt, and mutation bypass uses `--force` rather than interactive confirmation. |
| 2. Structured, parseable output | Complete | Core, diagnostic, setup, cache/model, daemon, profile, feedback, and dry-run surfaces expose `--json`; JSONL is limited to result streams. |
| 3. Errors that teach and enumerate | Complete | CLI enum parsing comes from Clap and MCP search mode/limit parsing rejects invalid values instead of silently defaulting. |
| 4. Safe retries and mutation boundaries | Complete | Cache/project clean, profile delete, init/install replacement, and daemon install flows use explicit `--dry-run` and/or `--force` contracts. |
| 5. Bounded responses | Complete | Search and file-list payloads include `limit`, `truncated`, warnings, and narrowing hints. |
| 6. Cross-CLI vocabulary consistency | Complete | Canonical vocabulary uses `source`, `filter-path`, `limit`, `list-files`, `get`, `status`, `--json`, `--force`, and `--dry-run`. |
| 7. Three-layer introspection | Complete | Human help, `sifs agent-context --json`, MCP `agent_context`, resources, and generated agent guidance cover the three layers. |
| 8. Async-aware execution | Partial by design | SIFS remains mostly synchronous; timeout/non-interactive behavior is implemented, while a durable jobs ledger is deferred until real job-shaped work exists. |
| 9. Persistent identity through profiles | Complete | `sifs profile` saves reusable source/search/model/cache defaults and exposes them through `agent-context` and MCP profile tools. |
| 10. Two-way I/O | Partial | Local-first `sifs feedback` and MCP feedback tools exist. Hosted/upstream feedback delivery is intentionally deferred. |

## Agent-facing entities

- Source: a local path or Git URL selected with `--source`, MCP `source`, or a
  profile.
- Profile: a saved source/search/model/cache context for repeated invocations.
- Index: the searchable representation for a source.
- Chunk: an indexed code/document span.
- Search request: query plus mode, limit, filters, and profile/source context.
- Search result: structured chunk match returned to CLI or MCP callers.
- Feedback entry: local JSONL record of agent friction.
- Agent file: `.claude/agents/sifs-search.md`.

## Canonical discovery surfaces

- Human help: `sifs --help` and subcommand help.
- Structured context: `sifs agent-context --json`.
- MCP context: `agent_context`, `sifs://agent/context`, `tools/list`, and
  `resources/list`.
- Workflow guidance: `src/agents/sifs-search.md` and MCP instructions.

## Prompt-native boundary

Prompt/spec-defined agent-facing behavior:

- MCP server instructions: `src/agents/mcp-instructions.md`
- MCP tool guidance: `src/agents/tools/*.md`
- MCP recovery messages: `src/agents/messages/*.md`
- Generated agent workflow: `src/agents/sifs-search.md`
- CLI/MCP discovery docs: `docs/cli.md`, `docs/mcp.md`

Code-defined engine mechanics:

- File walking and ignored directories.
- Chunking and language detection.
- Embedding model loading.
- BM25, dense search, hybrid ranking, and reranking.
- Cache validation and JSON-RPC framing.

## Verification commands

Run these before claiming this scorecard is current:

```bash
cargo test
cargo run --bin sifs -- agent-context --json
cargo run --bin sifs -- search "agent context" --source . --mode bm25 --json
cargo run --bin sifs -- mcp doctor --source . --offline --no-cache --json
```
