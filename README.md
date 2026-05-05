<p align="center">
  <img alt="SIFS Is Fast Search" src="assets/logo/sifs-logo.png" width="220">
</p>

# SIFS

Fast Rust code search for agents.

SIFS indexes repositories in `6.5ms` on average, answers normal warm searches
in `0.376ms`, and reaches `NDCG@10=0.8641` on the current annotated
63-repository benchmark corpus. It runs as a CLI, a Rust crate, or a local MCP
server for agent clients. Hybrid search combines semantic retrieval with BM25;
BM25 mode runs fully offline without loading model files or touching the
network.

[Quickstart](#quickstart) | [Features](#features) | [Install](#install) | [MCP server](#mcp-server) | [CLI](#cli) | [Rust library](#rust-library) | [How it works](#how-it-works) | [Benchmarks](#benchmarks)

## Quickstart

Install the public `sifs` binary, then search a project.

```bash
cargo install --locked sifs
sifs search "authentication flow" /path/to/project
sifs search "parse JWT claims" /path/to/project --mode bm25 --offline -k 10
sifs find-related src/auth/session.rs 42 /path/to/project -k 8
```

The default path is the current directory, and the default mode is `hybrid`.

```bash
sifs search "where is the login redirect handled"
```

## Features

- Search local code with natural-language queries, identifiers, or mixed
  architecture questions.
- Choose `hybrid`, `semantic`, or `bm25` ranking per query.
- Run BM25 search offline without tokenizers, safetensors, or Hugging Face model
  files.
- Use the same engine from the CLI, Rust, or an MCP client.
- Index local directories or shallow-cloned Git repositories.
- Generate agent files and run benchmark diagnostics for quality and latency
  checks.

## Install

Install from crates.io.

```bash
cargo install --locked sifs
```

Install with Homebrew from the public tap.

```bash
brew install tristanmanchester/tap/sifs
```

Build from source when you are working inside this repository.

```bash
cargo build --release
target/release/sifs search "authentication flow" .
```

The `sifs-benchmark` and `sifs-embed` binaries are diagnostics. Build them with
the explicit `diagnostics` feature.

```bash
cargo build --release --features diagnostics --bins
```

Run the test suite after changing indexing, chunking, ranking, model loading, or
MCP behavior.

```bash
cargo test
```

## MCP server

SIFS can install itself as a local stdio MCP server for Codex and Claude Code.
Install the binary once, start the shared daemon, then configure agent clients.

```bash
sifs daemon install-agent
sifs mcp install --client all
```

This installs a reusable MCP server instead of pinning the config to one
repository. Agent clients can ask SIFS to search the current project, and tool
calls can pass `repo` when they need a specific local checkout or Git URL.

You can still pin an MCP server to one source when you want that behavior.

```bash
sifs mcp install --client all --source /path/to/project
sifs mcp install --client codex --source /path/to/project
sifs mcp install --client claude --scope local --source /path/to/project
```

You can also start the server directly. Without a path it uses the server
process working directory as the default source. Passing a path pre-indexes that
source, so MCP clients can call `search` and `find_related` without sending a
`repo` argument on every tool call.

```bash
sifs mcp
sifs mcp /path/to/project
```

The installer uses the client CLIs when available.

```bash
codex mcp add sifs -- /absolute/path/to/sifs mcp
claude mcp add-json sifs '{"type":"stdio","command":"/absolute/path/to/sifs","args":["mcp"],"env":{}}' --scope local
```

If a client CLI is not available, `sifs mcp install --dry-run` prints fallback
config.

Codex uses `~/.codex/config.toml`:

```toml
[mcp_servers.sifs]
command = "/absolute/path/to/sifs"
args = ["mcp"]
startup_timeout_sec = 20
tool_timeout_sec = 60
```

Claude Code project config uses `.mcp.json`:

```json
{
  "mcpServers": {
    "sifs": {
      "type": "stdio",
      "command": "/absolute/path/to/sifs",
      "args": ["mcp"],
      "env": {}
    }
  }
}
```

This is a local process with read access to local paths provided in tool calls.
Only check project-scoped Claude Code `.mcp.json` into repositories you trust.

The daemon can also be run manually for debugging.

```bash
sifs daemon run --replace-existing-socket
sifs daemon ping
sifs daemon status --json
```

## CLI

Use `sifs search` for direct search. Use `sifs find-related` when you already
have a file and line and want similar code elsewhere in the same index.

```bash
# Search the current directory
sifs search "where is authentication handled"

# Search a local project with hybrid ranking
sifs search "parse oauth callback" /path/to/project --mode hybrid -k 10

# Use model-free offline BM25 search
sifs search "SessionToken" /path/to/project --mode bm25 --offline -k 10

# Search a remote Git repository
sifs search "stream upload backpressure" https://github.com/owner/project

# Find code related to a known location
sifs find-related src/auth/session.rs 42 /path/to/project -k 8
```

Use `--json`, `--jsonl`, or `--format` for structured output. Use
`--language`, `--path`, and `--context-lines` when an agent needs narrower
results.

Persistent index caches live in platform cache directories by default:
`~/Library/Caches/sifs` on macOS and `${XDG_CACHE_HOME:-~/.cache}/sifs` on
Linux. Use `--cache-dir` for another cache root, `--no-cache` to disable
persistent caches, or `--project-cache` to opt into a repository-local `.sifs/`
cache.

See [command-line usage](docs/cli.md) for every command and flag.

## Rust library

The crate exposes the same engine used by the CLI and MCP server.

```rust
use sifs::{SearchMode, SearchOptions, SifsIndex};

fn main() -> anyhow::Result<()> {
    let index = SifsIndex::from_path("/path/to/project")?;
    let results = index.search_with(
        "where is authentication handled",
        &SearchOptions::new(5).with_mode(SearchMode::Hybrid),
    )?;

    for result in results {
        println!("{} {}", result.chunk.location(), result.score);
    }

    Ok(())
}
```

Use `SifsIndex::from_path_sparse` for BM25-only indexes that never initialize
semantic state. Use `SifsIndex::from_git` to clone and index a remote
repository. See [Rust library usage](docs/library.md) for model policy, filters,
custom extensions, and chunk-level construction.

## How it works

SIFS walks repositories with `.gitignore`-aware file selection, splits files
into useful code chunks, builds a sparse BM25 index, and keeps semantic state
lazy until a semantic or hybrid query needs it.

Search modes:

- `bm25`: sparse lexical search for identifiers, symbols, and exact terms.
- `semantic`: embedding similarity with the configured encoder.
- `hybrid`: semantic and BM25 rankings fused together, then reranked.

The default semantic encoder is `minishlab/potion-code-16M` through a local
Model2Vec loader. The loader reads model tensors and tokenizer files directly,
so the query path stays inside the Rust process after the model is available
locally.

Hybrid search over-fetches candidates, normalizes each ranking with reciprocal
rank fusion, applies query-aware boosts, and reranks the top results. Symbol-like
queries lean more heavily on BM25. Natural-language questions keep more semantic
weight.

Use `sifs model pull` or `sifs model fetch` to prefetch the default model. Use
`sifs doctor` to check whether semantic search is ready for offline use.

## Benchmarks

The current full-corpus benchmark uses 63 pinned open-source repositories, 19
languages, and 1,251 annotated search tasks. SIFS reaches `NDCG@10=0.8641`,
indexes cold in `6.5ms`, and answers normal warm queries in `0.376ms`.

![SIFS search quality versus warm uncached query latency](assets/images/quality_vs_warm_latency.png)

The comparison table is regenerated from benchmark JSON by
[benchmarks/plot_sifs_comparison.py](benchmarks/plot_sifs_comparison.py). The
Semble row is included as a direct comparison to the Python tool; the SIFS
headline avoids using that tool's name for the corpus itself.

| Method | NDCG@10 | Cold index | Warm uncached query | Cached repeat query |
|---|---:|---:|---:|---:|
| **SIFS** | **0.8641** | **6.5 ms** | **0.376 ms** | **0.0012 ms** |
| CodeRankEmbed Hybrid | 0.8617 | 57.3 s | 16.9 ms | n/a |
| Semble | 0.8544 | 439.4 ms | 1.3 ms | n/a |
| CodeRankEmbed | 0.7648 | 57.3 s | 13.3 ms | n/a |
| ColGREP | 0.6925 | 3.9 s | 979.3 ms | n/a |
| grepai | 0.5606 | 35.0 s | 47.7 ms | n/a |
| probe | 0.3872 | 0.0000 ms | 207.1 ms | n/a |
| ripgrep | 0.1257 | 0.0000 ms | 8.8 ms | n/a |

SIFS reports three timing fields so repeated-query caching is not mixed up with
normal search speed:

```text
cold_index_ms
warm_uncached_query_ms
warm_cached_repeat_query_ms
```

Use `warm_uncached_query_ms` when comparing normal searches after an index
exists. Use `warm_cached_repeat_query_ms` only for repeated identical queries
inside the same process. The older `0.0017ms` figure was cached repeat latency,
not general warm-query latency.

SIFS also measures how quickly relevant files enter an agent's context. The
curve below counts annotated relevant target files as retrieved chunks are added
to the prompt budget.

![SIFS context efficiency: recall versus retrieved context tokens](assets/images/context_efficiency_comparison.png)

SIFS is strongest on symbol-heavy queries while still doing well on semantic
and architecture questions.

| Query type | NDCG@10 |
|---|---:|
| symbol | 0.9437 |
| semantic | 0.8551 |
| architecture | 0.8313 |

![SIFS quality by query type and search mode](assets/images/query_type_quality_by_mode.png)

The benchmark artifacts live in [benchmarks/results](benchmarks/results). The
full methodology, per-language breakdown, extra figures, and React
large-repository smoke result are in
[docs/benchmark-report.md](docs/benchmark-report.md).

## Documentation

- [Command-line usage](docs/cli.md): `search`, `find-related`, `init`, model
  commands, cache commands, and MCP server startup.
- [Rust library usage](docs/library.md): `SifsIndex`, search modes, filters, and
  indexing options.
- [MCP server usage](docs/mcp.md): stdio protocol behavior and tool schemas.
- [Agent-native scorecard](docs/agent-native-scorecard.md): the agent-facing
  contract and readiness evidence.
- [Benchmarking](docs/benchmarks.md): quality, latency, embedding, and local
  smoke benchmarks.
- [Architecture](docs/architecture.md): file selection, chunking, embedding,
  sparse search, dense search, and hybrid ranking.

## File coverage

By default, SIFS indexes code files and skips common generated, dependency, and
cache directories. It uses the `ignore` crate, so nested `.gitignore` files, Git
excludes, global Git ignores, and hidden files behave like familiar developer
search tools. Text-like documents such as Markdown, YAML, TOML, and JSON are
available through library options.

The file walker currently recognizes Python, JavaScript, TypeScript, Go, Rust,
Java, Kotlin, Ruby, PHP, C, C++, C#, Swift, Scala, Elixir, Dart, Lua, SQL, Bash,
Zig, Haskell, Markdown, YAML, TOML, and JSON extensions.
