# MCP server usage

SIFS can run as a Model Context Protocol server over standard input and output.
This mode keeps indexes cached inside one process, which makes repeated agent
searches much faster than rebuilding the index for each direct CLI command.

## Start the server

Run `sifs` without a subcommand to start stdio server mode. You can pass a
default local path or Git URL as the first positional argument.

```bash
target/release/sifs /path/to/project
```

When the default source is a Git URL, use `--ref` to choose a branch or tag.

```bash
target/release/sifs https://github.com/owner/project --ref main
```

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
args = ["/path/to/project"]
```

For Git-backed search, pass the Git URL and optional ref as arguments.

```toml
[mcp_servers.sifs]
command = "/absolute/path/to/sifs"
args = ["https://github.com/owner/project", "--ref", "main"]
```

## Protocol surface

The server uses JSON-RPC messages with `Content-Length` framing. It handles the
standard MCP initialization and tool-list methods that clients call during
startup.

Supported methods are:

- `initialize`: Returns server metadata, tool capability information, and usage
  instructions.
- `tools/list`: Returns the `search` and `find_related` tool schemas.
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

Use `hybrid` for most tasks. Use `bm25` for exact symbols and `semantic` for
meaning-only exploration.

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

## Index caching

The MCP server caches indexes for the lifetime of the process. Local sources
are keyed by canonical path. Git sources are keyed by URL and optional ref.

The first call for a source pays the indexing cost. Later calls reuse the
in-memory `SifsIndex`, including the loaded model, BM25 index, dense vectors,
and chunk mappings.

## Error handling

Tool calls return text content even when indexing or lookup fails. This keeps
the response shape simple for clients and agents.

Common errors include:

- Missing `repo` when the server has no default source.
- A local path that doesn't exist or isn't a directory.
- A Git clone failure.
- A file and line that don't map to an indexed chunk.

## Next steps

Read [Command-line usage](cli.md) for direct terminal commands, or read
[Architecture](architecture.md) to understand what the server caches.
