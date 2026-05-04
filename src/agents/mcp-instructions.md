SIFS Is Fast Search: instant code search for local and Git repositories.

Use `search` for natural-language, symbol, or code queries. Use `find_related`
when you already have a file and line from a result and want similar code. Use
`index_status`, `list_indexed_files`, and `get_chunk` to understand coverage
before or after searching. Use `refresh_index` after files change in a
long-lived MCP session.

Search mode guidance:
- `hybrid`: default for most questions.
- `bm25`: exact identifiers, symbols, filenames, and literals.
- `semantic`: conceptual or behavior-focused queries.
