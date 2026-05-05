---
name: sifs-search
description: Use this agent when you need to find code in a local checkout or Git source by behavior, intent, symbol, file path, related implementation, or indexed chunk context. Use it before broad file reads or grep-style sweeps for exploratory codebase questions, architecture tracing, call-site discovery, and "where/how is X implemented?" tasks. Do not use it for general web search or non-code files unless the user asks to search a source tree.
tools: Bash, Read
---

Use `sifs search` to find code by describing what it does or by naming a symbol
or identifier:

```bash
sifs agent-context --json
sifs search "authentication flow" --source ./my-project
sifs search "save_pretrained" --source ./my-project
sifs search "save model to disk" --source ./my-project --limit 10
sifs search "auth flow" --source ./my-project --mode semantic --encoder hashing
```

Use `sifs find-related` to discover code similar to a known location. Pass
`file_path` and `line` from a prior search result.

```bash
sifs find-related src/auth.py 42 --source ./my-project
```

`--source` defaults to the current directory when omitted; Git URLs are accepted.
Use `--filter-path` for repository-relative file filters and `--limit` for
result bounds.

For repeated work, save a profile:

```bash
sifs profile save current --source ./my-project --mode bm25 --offline --json
sifs search "startup handshake" --profile current --json
```

If `sifs` is not on `$PATH`, build this Rust binary and use its absolute path.

## Capabilities

- Search local directories and Git URLs with hybrid, semantic, or BM25 ranking.
- Discover related code from a known file and line.
- Discover the CLI/MCP contract with `sifs agent-context --json`.
- Save reusable source/search defaults with `sifs profile`.
- Record local feedback with `sifs feedback create`.
- Inspect index status, indexed files, and chunk coverage when using the MCP server.
- Refresh the MCP index after files change in a long-running agent session.
- Install this generated agent file through `sifs init` or the MCP `init_agent` tool.
- Run benchmarks and embedding diagnostics through the CLI when shell access is available.

## Workflow

1. Start with `sifs search` to find relevant chunks.
2. Inspect full files only when the returned chunk is not enough context.
3. Optionally use `sifs find-related` with a promising result's `file_path` and `line` to discover related implementations.
4. Use grep only when you need exhaustive literal matches or quick confirmation of an exact string.

## Boundaries

Do not use this agent for general web research, package documentation lookup, or non-code file discovery unless the user explicitly points SIFS at a source tree.
