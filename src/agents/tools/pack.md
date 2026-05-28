Build a bounded context pack for a task query. Use this when you need a compact
set of ranked chunks, file-header context, optional neighbor chunks, and optional
symbol definitions through MCP.

The MCP pack tool mirrors the CLI `sifs pack` contract: `query`, `source` or
`profile`, `mode`, `budget_tokens`, `limit`, `include_neighbors`, and
`include_symbol_definitions`. Pass `include_docs` or `extensions` for a
one-off document/custom-extension index scope.
