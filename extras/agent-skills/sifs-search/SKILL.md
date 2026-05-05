---
name: sifs-search
description: Find code by intent, symbol, file path, behavior, or related implementation using the SIFS CLI. Prefer this over raw grep for exploratory codebase search.
---

Use SIFS from the shell when you need codebase context. The CLI is the reliable path; MCP tools are optional and should only be used when they are visible in the current agent session.

Start by discovering the local contract:

```bash
sifs agent-context --json
```

Search by intent, behavior, symbol, or exact text:

```bash
sifs search "authentication flow" --source <project>
sifs search "save_pretrained" --source <project> --mode bm25
sifs search "save model to disk" --source <project> --limit 10
```

Inspect indexed files and chunks before reading broad files:

```bash
sifs list-files --source <project> --limit 200 --json
sifs get src/auth.rs 42 --source <project>
sifs find-related src/auth.rs 42 --source <project>
```

Use `--source <project>` when the agent may not be running from the target checkout. Use `--filter-path <repo-relative-path>` for path narrowing and `--limit` for bounded results.

If MCP tools named `search`, `get_chunk`, or `list_files` are visible, they may be used for the same workflow. If they are missing, configured-but-invisible, or failing, fall back to the CLI immediately.
