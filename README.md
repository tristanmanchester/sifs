# SIFS

SIFS means "SIFS Is Fast Search." It's a Rust-native code search tool and
library for agents, editor integrations, and local developer workflows. It
indexes a repository, splits files into useful chunks, embeds those chunks with
a Model2Vec-compatible encoder, and serves fast hybrid search over semantic and
BM25 rankings.

## What SIFS does

SIFS turns a local directory or Git repository into a searchable code index. You
can use it as a command-line tool, as a Rust crate, or as a Model Context
Protocol server for agent clients.

- Search code with natural-language or symbol-heavy queries.
- Find chunks related to a specific file and line.
- Index local directories or shallow-cloned Git repositories.
- Use hybrid, semantic-only, or BM25-only ranking.
- Run quality and latency benchmarks over annotated repositories.

## Build SIFS

SIFS builds with Cargo. The release build gives you the `sifs`,
`sifs-benchmark`, and `sifs-embed` binaries under `target/release/`.

```bash
cargo build --release
```

Run the test suite after changing indexing, chunking, ranking, or model-loading
behavior.

```bash
cargo test
```

## Quick start

Use `sifs search` for direct command-line search. The default path is the
current directory and the default mode is `hybrid`.

```bash
target/release/sifs search "authentication flow" /path/to/project
target/release/sifs search "parse JWT claims" /path/to/project --mode bm25 -k 10
```

Use `sifs find-related` when you already have a location and want similar code
elsewhere in the same index.

```bash
target/release/sifs find-related src/auth/session.rs 42 /path/to/project -k 8
```

Start the MCP server by running `sifs` without a subcommand. Passing a path
pre-indexes that source and lets MCP clients call `search` and `find_related`
without including a `repo` argument on every tool call.

```bash
target/release/sifs /path/to/project
```

## Documentation

The documentation is split by usage surface. Start with the page that matches
how you plan to use SIFS.

- [Command-line usage](docs/cli.md) covers `search`, `find-related`, `init`,
  and MCP server startup.
- [Rust library usage](docs/library.md) covers `SifsIndex`, search modes,
  filters, and indexing options.
- [MCP server usage](docs/mcp.md) covers the stdio protocol surface and tool
  schemas.
- [Benchmarking](docs/benchmarks.md) covers quality, latency, embedding, and
  local smoke benchmarks.
- [Architecture](docs/architecture.md) explains file selection, chunking,
  embedding, sparse search, dense search, and hybrid ranking.

## Search model

SIFS uses `minishlab/potion-code-16M` by default through a local Model2Vec
loader. The loader reads the model tensors and tokenizer files directly, so the
query path stays inside the Rust process after the model is available locally.

Hybrid search combines semantic and BM25 rankings. It over-fetches candidates,
normalizes each ranking with reciprocal rank fusion, applies query-aware boosts,
and reranks the top results. Symbol-like queries lean more heavily on BM25,
while natural-language queries keep more semantic weight.

## File coverage

By default, SIFS indexes code files and skips common generated, dependency, and
cache directories. It respects the root `.gitignore` file and supports text-like
documents such as Markdown, YAML, TOML, and JSON through the library options.

The file walker currently recognizes Python, JavaScript, TypeScript, Go, Rust,
Java, Kotlin, Ruby, PHP, C, C++, C#, Swift, Scala, Elixir, Dart, Lua, SQL,
Bash, Zig, Haskell, Markdown, YAML, TOML, and JSON extensions.

## Next steps

Read [command-line usage](docs/cli.md) if you want to run SIFS locally, or read
[MCP server usage](docs/mcp.md) if you want to wire it into an agent client.
