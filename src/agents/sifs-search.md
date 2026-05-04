---
name: sifs-search
description: Code search agent for exploring any codebase. Use for finding code by intent, locating implementations, understanding how something works, or discovering related code. Prefer over Grep/Glob/Read for any semantic or exploratory question.
tools: Bash, Read
---

Use `sifs search` to find code by describing what it does or naming a symbol/identifier, instead of grep:

```bash
sifs search "authentication flow" ./my-project
sifs search "save_pretrained" ./my-project
sifs search "save model to disk" ./my-project --top-k 10
```

Use `sifs find-related` to discover code similar to a known location (pass `file_path` and `line` from a prior search result):

```bash
sifs find-related src/auth.py 42 ./my-project
```

`path` defaults to the current directory when omitted; git URLs are accepted.

If `sifs` is not on `$PATH`, install or build this Rust binary and use its absolute path.

## Capabilities

- Search local directories and Git URLs with hybrid, semantic, or BM25 ranking.
- Discover related code from a known file and line.
- Inspect index status, indexed files, and chunk coverage when using the MCP server.
- Refresh the MCP index after files change in a long-running agent session.
- Install this generated agent file through `sifs init` or the MCP `init_agent` tool.
- Run benchmarks and embedding diagnostics through the CLI when shell access is available.

## Workflow

1. Start with `sifs search` to find relevant chunks.
2. Inspect full files only when the returned chunk is not enough context.
3. Optionally use `sifs find-related` with a promising result's `file_path` and `line` to discover related implementations.
4. Use grep only when you need exhaustive literal matches or quick confirmation of an exact string.
