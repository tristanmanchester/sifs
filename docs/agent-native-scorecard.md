# Agent-Native Scorecard

This scorecard defines SIFS' agent-native contract and the evidence used to
evaluate it. SIFS is a deterministic code search engine with an agent-facing MCP
workspace. The agent-native surface is the CLI, MCP server, generated agent
file, and documented Rust API. Low-level search internals such as BM25 postings,
dense vectors, cache signatures, and chunking heuristics are implementation
details rather than user-editable workspace entities.

## Scoring Scope

First-class agent-facing entities:

- Source: a local path or Git URL selected for search.
- Index: the searchable representation for a source.
- Chunk: an indexed code/document span.
- Search request: query plus ranking options and filters.
- Search result: structured chunk match returned to the agent.
- Agent file: `.claude/agents/sifs-search.md`.
- Benchmark run: a CLI evaluation job and output artifact.
- Embedding diagnostic: a CLI text-to-vector check.

Implementation-only internals:

- `Bm25Index`, `DenseIndex`, `FileSignature`, search caches, ranking constants,
  tokenizer/model tensors, JSON-RPC framing, and Tree-sitter chunk mechanics.

These internals are intentionally code-defined because they require deterministic
behavior, precise tests, and stable performance. They are not counted as CRUD or
prompt-native feature entities.

## Overall Score Summary

| Core Principle | Score | Percentage | Status | Evidence |
|----------------|-------|------------|--------|----------|
| Action Parity | 33/33 | 100% | Complete | CLI/library actions are shell- or Rust-composable; MCP covers core search, index inspection, refresh, chunk read, and agent-file creation. |
| Tools as Primitives | 8/8 | 100% | Complete | MCP tools expose primitive capabilities: search, related lookup, status, refresh, clear, file list, chunk read, and agent install. |
| Context Injection | 10/10 | 100% | Complete | Dynamic instructions, MCP resources, index status, indexed file inventory, structured results, default source/ref, cache keys, capabilities, and prompt/spec guidance are exposed. |
| Shared Workspace | 10/10 | 100% | Complete | Local paths share the same source tree without dirtying it by default; platform cache is default, repo-local `.sifs` is opt-in, MCP refresh/clear handles long-running sessions, and Git URL isolation is explicit. |
| CRUD Completeness | 8/8 | 100% | Complete | Each first-class agent-facing entity has create/read/update/delete or a documented non-mutating equivalent. |
| UI Integration | 10/10 | 100% | Complete | CLI/MCP actions return visible output, structured content, status, refresh feedback, benchmark progress, and output-write confirmation. |
| Capability Discovery | 7/7 | 100% | Complete | README/docs, CLI help, `sifs capabilities`, MCP `tools/list`, MCP resources, generated agent capabilities, and empty-state guidance are present. |
| Prompt-Native Features | 8/8 | 100% | Complete | Agent-facing behavior lives in prompt/spec docs; deterministic engine mechanics remain code-defined by design. |

## CRUD Completeness

| Entity | Create | Read | Update | Delete / Clear |
|--------|--------|------|--------|----------------|
| Source | CLI/MCP `repo` or default source | `index_status` | server restart/config update | `clear_index` removes source from memory cache |
| Index | implicit first search/status build | `index_status` | `refresh_index` | `clear_index` |
| Chunk | created by indexing | `get_chunk`, `search`, `find_related` | `refresh_index` from source changes | `clear_index` or source deletion followed by refresh |
| Search request | `search` tool/CLI invocation | structured response metadata | rerun with changed options | no persisted request state |
| Search result | produced by `search`/`find_related` | structured content and text output | rerun with changed source/options | no persisted result state |
| Agent file | `sifs init`, MCP `init_agent` | filesystem/shared workspace | `--force` or `force=true` | filesystem/shared workspace |
| Benchmark run | `sifs-benchmark` CLI | stdout or output JSON | rerun with changed flags | remove output artifact |
| Embedding diagnostic | `sifs-embed` CLI | stdout JSON vector | rerun with changed text/model | no persisted state |

Search requests, search results, benchmark runs, and embedding diagnostics are
events rather than stored records. Their delete operation is therefore "no
persisted state" or deletion of the explicitly written artifact.

## Prompt-Native Feature Boundary

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

This boundary is intentional: prompt/spec files define outcomes and agent
behavior; Rust code preserves deterministic retrieval mechanics.

## Verification Commands

Run these before claiming this scorecard is current:

```bash
cargo test
cargo run --bin sifs -- --help
cargo run --bin sifs -- search --help
cargo run --bin sifs -- find-related --help
cargo run --bin sifs -- capabilities
```
