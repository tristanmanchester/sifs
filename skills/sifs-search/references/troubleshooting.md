# SIFS Troubleshooting

## `sifs` is not found

Check the binary:

```bash
command -v sifs
sifs --version
```

If the binary is not on `PATH`, use the absolute binary path or build/install SIFS first.

## Search runs against the wrong checkout

Pass the source explicitly:

```bash
sifs search "query" --source <project>
```

Global skills should stay ambient. Project-specific snippets may pin a source only when that was explicitly requested during install.

## Semantic search is unavailable or slow

Use BM25 for exact names and offline work:

```bash
sifs search "ExactSymbol" --source <project> --mode bm25 --offline
```

## MCP is configured but not visible

Use CLI fallback and run:

```bash
sifs agent doctor --target codex --json
sifs mcp doctor --source <project> --offline --no-cache --json
```

Visibility can be `unknown` from the CLI because the active agent session controls exposed tools.

## Installed instructions are stale

Run doctor and reinstall the managed artifact:

```bash
sifs agent doctor --target codex --json
sifs agent install --target codex --artifact snippet --file AGENTS.md --force
```
