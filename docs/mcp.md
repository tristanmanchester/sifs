# MCP server usage

SIFS can run as a Model Context Protocol server over standard input and output.
This mode keeps indexes cached inside one process, which makes repeated agent
searches much faster than rebuilding the index for each direct CLI command.

## Start the server

Run `sifs mcp` to start stdio server mode. You can pass a default local path or
Git URL as the first positional argument.

```bash
target/release/sifs mcp /path/to/project
```

When the default source is a Git URL, use `--ref` to choose a branch or tag.

```bash
target/release/sifs mcp https://github.com/owner/project --ref main
```

Use `--model`, `--no-download`, or `--offline` to control semantic model
loading for server mode. `--offline` also rejects Git URL sources.

When you pass a default source, MCP tool calls are scoped to that source and can
omit `repo`. Calls that pass a different `repo` are rejected. If you don't pass
a default source, MCP tool calls must include `repo` with a local path or Git
URL.

## Configure a client

Most MCP clients let you configure a command and argument list. Use an absolute
path to the release binary when the client doesn't run from the SIFS repository.

```toml
[mcp_servers.sifs]
command = "/absolute/path/to/sifs"
args = ["mcp", "/path/to/project"]
```

For Git-backed search, pass the Git URL and optional ref as arguments.

```toml
[mcp_servers.sifs]
command = "/absolute/path/to/sifs"
args = ["mcp", "https://github.com/owner/project", "--ref", "main"]
```

## Protocol surface

The server uses JSON-RPC messages with `Content-Length` framing. It handles the
standard MCP initialization and tool-list methods that clients call during
startup.

Supported methods are:

- `initialize`: Returns server metadata, tool capability information, and usage
  instructions. The instructions include the configured default source/ref and
  the repo override policy when a default source is present.
- `resources/list`: Returns context resources for server state and index
  discovery.
- `resources/read`: Reads context resources such as `sifs://server/context`.
- `tools/list`: Returns schemas for search, related-code discovery, index
  inspection, refresh, cache clearing, indexed-file listing, and chunk reading.
- `tools/call`: Runs a supported tool and returns text content.

Unsupported methods return an error string inside the JSON-RPC result payload.

## Search tool

Use the `search` tool to find relevant chunks in a repository. The query can be
natural language, code, an identifier, or a short phrase.

Input schema:

```json
{
  "query": "Natural language or code query.",
  "repo": "/path/to/project",
  "mode": "hybrid",
  "top_k": 5
}
```

Fields:

- `query` is required.
- `repo` is optional when the server started with a default source. When a
  default source is configured, `repo` must be omitted or match that source.
- `mode` is optional and can be `hybrid`, `semantic`, or `bm25`.
- `top_k` is optional and defaults to `5`.
- `alpha` is optional for hybrid search. Omit it to let SIFS choose the blend
  from the query shape.
- `filter_languages` is optional and accepts exact language labels such as
  `rust`.
- `filter_paths` is optional and accepts repository-relative file paths.

Use `hybrid` for most tasks. Use `bm25` for exact symbols and `semantic` for
meaning-only exploration.

Search responses include text content for agent context injection and
`structuredContent` with source metadata, index stats, and result objects. Each
structured result includes `file_path`, `start_line`, `end_line`, `language`,
`score`, `source`, and `content`.

## Find-related tool

Use the `find_related` tool to locate chunks similar to a known file and line.
The file path must match an indexed chunk path or resolve to one.

Input schema:

```json
{
  "file_path": "src/auth/session.rs",
  "line": 42,
  "repo": "/path/to/project",
  "top_k": 5
}
```

Fields:

- `file_path` is required.
- `line` is required and uses one-based line numbers.
- `repo` is optional when the server started with a default source. When a
  default source is configured, `repo` must be omitted or match that source.
- `top_k` is optional and defaults to `5`.

When SIFS can't resolve the file and line, the tool returns a text error that
asks you to check whether the file is indexed and the line is inside a known
chunk.

## Index inspection tools

Use `index_status` to inspect the selected repository before or after searching.
It returns the source, optional Git ref, memory-cache state, indexed file count,
chunk count, language distribution, and available MCP tools.

```json
{
  "repo": "/path/to/project"
}
```

Use `list_indexed_files` to see which repository-relative file paths are in the
index.

```json
{
  "repo": "/path/to/project",
  "limit": 200
}
```

Use `get_chunk` to read the indexed chunk containing a known file and one-based
line.

```json
{
  "file_path": "src/auth/session.rs",
  "line": 42,
  "repo": "/path/to/project"
}
```

Use `refresh_index` after a user or agent edits files while a long-running MCP
server is active. The server keeps in-memory indexes for the process lifetime,
so refresh is the explicit way to make search reflect recent file changes
without restarting the server.

Use `clear_index` to remove a source from the in-memory cache. The next search
or status call rebuilds or reloads the index.

Use `init_agent` to create the generated SIFS Claude agent file from an MCP
client. It defaults to `.claude/agents/sifs-search.md` and accepts `force` to
overwrite an existing file.

```json
{
  "destination": ".claude/agents/sifs-search.md",
  "force": false
}
```

## Resources

The server exposes lightweight MCP resources for discovery:

- `sifs://server/context`: server version, default source/ref, cache keys, tools,
  and resource URIs.
- `sifs://index/status`: pointer to the `index_status` tool for current stats.
- `sifs://index/files`: pointer to the `list_indexed_files` tool for indexed
  file inventory.

## Index caching

The MCP server caches indexes for the lifetime of the process. Local sources
are keyed by canonical path. Git sources are keyed by URL and optional ref.

The first call for a source pays the indexing cost. Later calls reuse the
in-memory `SifsIndex`, including the BM25 index and chunk mappings. Semantic
model state and dense vectors are loaded lazily only after semantic, hybrid, or
`find_related` calls. BM25 tool calls stay model-free.

If files change while the MCP server keeps running, call `refresh_index` before
trusting search results for the changed source. Git URL mode indexes a temporary
clone, so it does not include uncommitted changes from a separate local checkout.

## Error handling

Tool calls return text content even when indexing or lookup fails. This keeps
the response shape simple for clients and agents.

Common errors include:

- Missing `repo` when the server has no default source.
- A local path that doesn't exist or isn't a directory.
- A Git clone failure.
- A semantic or hybrid search when the requested model is unavailable under
  `--offline` or `--no-download`.
- A file and line that don't map to an indexed chunk.

## Next steps

Read [Command-line usage](cli.md) for direct terminal commands, or read
[Architecture](architecture.md) to understand what the server caches.
