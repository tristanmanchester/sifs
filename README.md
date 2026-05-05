<p align="center">
  <img alt="SIFS Is Fast Search" src="assets/logo/sifs-logo.png" width="220">
</p>

<h2 align="center">Fast Code Search for Agents</h2>

<p align="center">
  <a href="https://crates.io/crates/sifs"><img src="https://img.shields.io/crates/v/sifs?color=%23007ec6&label=crates.io" alt="Crates.io version"></a>
  <a href="https://github.com/tristanmanchester/sifs/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-green" alt="License - MIT"></a>
</p>

<p align="center">
  <a href="#quickstart">Quickstart</a> •
  <a href="#mcp-server">MCP Server</a> •
  <a href="#cli">CLI</a> •
  <a href="#rust-library">Rust Library</a> •
  <a href="#benchmarks">Benchmarks</a>
</p>

SIFS indexes a repo in **6.5 ms**, answers queries in **0.376 ms**, and hits **NDCG@10 = 0.8641**, beating every other tool on the benchmark, including the 137M-parameter CodeRankEmbed Hybrid. It runs as a CLI, a Rust crate, or a local MCP server. No GPU, no API keys, no external services.

## Quickstart

```bash
cargo install --locked sifs
sifs search "authentication flow" /path/to/project
```

That's it. The default mode is `hybrid` (semantic + BM25). For a fully offline, model-free search, drop in `--mode bm25`:

```bash
sifs search "parse JWT claims" /path/to/project --mode bm25 --offline -k 10
```

To find code similar to a known location:

```bash
sifs find-related src/auth/session.rs 42 /path/to/project -k 8
```

The `path` argument defaults to the current directory, and git URLs work anywhere a path does.

## Features

- **Fastest in class.** 6.5 ms cold index, 0.376 ms warm query, 0.0012 ms for cached repeats. Pure Rust, all on CPU.
- **State-of-the-art quality.** NDCG@10 of 0.8641 across 63 repositories and 19 languages. Ahead of CodeRankEmbed Hybrid (0.8617) and Semble (0.8544).
- **Three search modes.** `hybrid` for most queries, `semantic` for natural language, `bm25` for symbols and identifiers. Switch per query.
- **Fully offline.** BM25 mode loads nothing — no tokenizers, no model files, no network. Hybrid and semantic modes work offline once the model is cached locally.
- **MCP server.** Drop-in tool for Claude Code, Codex, Cursor, and any other MCP-compatible agent. Repos are indexed on demand; local paths are watched for changes.
- **Local and remote.** Pass a local path or a git URL.

## Install

```bash
# crates.io
cargo install --locked sifs

# Homebrew
brew install tristanmanchester/tap/sifs

# From source
cargo build --release
target/release/sifs search "authentication flow" .
```

The `sifs-benchmark` and `sifs-embed` diagnostic binaries require the `diagnostics` feature:

```bash
cargo build --release --features diagnostics --bins
```

Run the test suite after changing indexing, chunking, ranking, model loading, or MCP behavior:

```bash
cargo test
```

## MCP Server

SIFS installs itself as a local stdio MCP server in two commands:

```bash
sifs daemon install-agent
sifs mcp install --client all
```

This gives every agent client a reusable server that can search any project. Tool calls can pass `repo` to target a specific local checkout or git URL, or omit it to search the client's working directory.

To pin the server to a single source:

```bash
sifs mcp install --client all --source /path/to/project
sifs mcp install --client codex --source /path/to/project
sifs mcp install --client claude --scope local --source /path/to/project
```

You can also start the server directly:

```bash
sifs mcp                      # uses the process working directory
sifs mcp /path/to/project     # pre-indexes that source on startup
```

The installer calls the client CLIs when they're available:

```bash
codex mcp add sifs -- /absolute/path/to/sifs mcp
claude mcp add-json sifs '{"type":"stdio","command":"/absolute/path/to/sifs","args":["mcp"],"env":{}}' --scope local
```

If a client CLI isn't available, `sifs mcp install --dry-run` prints the config to paste manually.

<details>
<summary><b>Manual config snippets</b></summary>

**Codex** (`~/.codex/config.toml`):
```toml
[mcp_servers.sifs]
command = "/absolute/path/to/sifs"
args = ["mcp"]
startup_timeout_sec = 20
tool_timeout_sec = 60
```

**Claude Code** (`.mcp.json` in your project):
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

Only check a project-scoped `.mcp.json` into repositories you trust — it grants read access to local paths passed in tool calls.

</details>

To debug the daemon directly:

```bash
sifs daemon run --replace-existing-socket
sifs daemon ping
sifs daemon status --json
```

## CLI

```bash
# Search the current directory
sifs search "where is authentication handled"

# Hybrid search with more results
sifs search "parse oauth callback" /path/to/project --mode hybrid -k 10

# Offline BM25 — no model files needed
sifs search "SessionToken" /path/to/project --mode bm25 --offline -k 10

# Remote repo (cloned on demand)
sifs search "stream upload backpressure" https://github.com/owner/project

# Find code near a known location
sifs find-related src/auth/session.rs 42 /path/to/project -k 8
```

Structured output: `--json`, `--jsonl`, or `--format`. Narrower results: `--language`, `--path`, `--context-lines`.

Index caches live in platform cache directories by default (`~/Library/Caches/sifs` on macOS, `${XDG_CACHE_HOME:-~/.cache}/sifs` on Linux). Override with `--cache-dir`, disable with `--no-cache`, or opt into a repo-local `.sifs/` cache with `--project-cache`.

Full CLI reference: [docs/cli.md](docs/cli.md).

## Rust Library

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

For BM25-only indexes that never touch semantic state, use `SifsIndex::from_path_sparse`. For remote repos, use `SifsIndex::from_git`. Full API docs, model policy, filters, and chunk-level construction: [docs/library.md](docs/library.md).

## How It Works

SIFS walks a repo using `.gitignore`-aware file selection, splits files into code chunks, builds a sparse BM25 index, and keeps semantic state lazy until a semantic or hybrid query actually needs it.

**`bm25`** — sparse lexical search. Good for identifiers, symbols, and exact terms. No model files required.

**`semantic`** — embedding similarity using `minishlab/potion-code-16M` through a local Model2Vec loader. The model tensors and tokenizer files are read directly into the Rust process; nothing leaves the machine after the initial download.

**`hybrid`** — the default. Semantic and BM25 rankings are fused with reciprocal rank fusion, then reranked. Symbol-like queries lean on BM25; natural-language questions keep more semantic weight.

<details>
<summary><b>Ranking signals</b></summary>

- **Query-aware mode weighting.** Symbol queries (`Foo::bar`, `getUserById`) get more BM25 weight. Natural-language queries stay balanced.
- **Definition boosts.** A chunk that defines the queried symbol (`class`, `fn`, `def`) ranks above chunks that only reference it.
- **Identifier stemming.** Query tokens are stemmed and matched against identifier stems, so `parse config` boosts chunks containing `parseConfig`, `ConfigParser`, or `config_parser`.
- **File coherence.** When multiple chunks from the same file match, the file is boosted so results reflect file-level relevance rather than a single out-of-context snippet.
- **Noise penalties.** Test files, `compat/`/`legacy/` shims, example code, and `.d.ts` stubs are down-ranked so canonical implementations surface first.

</details>

Use `sifs model pull` or `sifs model fetch` to pre-download the default model. Use `sifs doctor` to confirm semantic search is ready for offline use.

## Benchmarks

Benchmarks run across 63 pinned open-source repositories, 19 languages, and 1,251 annotated search tasks.

![SIFS search quality versus warm uncached query latency](assets/images/quality_vs_warm_latency.png)

| Method | NDCG@10 | Cold index | Warm query | Cached repeat |
|---|---:|---:|---:|---:|
| **SIFS** | **0.8641** | **6.5 ms** | **0.376 ms** | **0.0012 ms** |
| CodeRankEmbed Hybrid | 0.8617 | 57.3 s | 16.9 ms | n/a |
| Semble | 0.8544 | 439.4 ms | 1.3 ms | n/a |
| CodeRankEmbed | 0.7648 | 57.3 s | 13.3 ms | n/a |
| ColGREP | 0.6925 | 3.9 s | 979.3 ms | n/a |
| grepai | 0.5606 | 35.0 s | 47.7 ms | n/a |
| probe | 0.3872 | — | 207.1 ms | n/a |
| ripgrep | 0.1257 | — | 8.8 ms | n/a |

SIFS reports three timing fields to avoid mixing up caching effects:

- `cold_index_ms` — fresh index, no cache
- `warm_uncached_query_ms` — normal query after index exists (use this for comparisons)
- `warm_cached_repeat_query_ms` — repeated identical query in the same process

### Quality by query type

SIFS is strongest on symbol queries but holds up well on semantic and architecture questions too.

| Query type | NDCG@10 |
|---|---:|
| symbol | 0.9437 |
| semantic | 0.8551 |
| architecture | 0.8313 |

![SIFS quality by query type and search mode](assets/images/query_type_quality_by_mode.png)

### Context efficiency

The chart below tracks how quickly annotated relevant files enter an agent's context as retrieved chunks are added to the prompt budget.

![SIFS context efficiency: recall versus retrieved context tokens](assets/images/context_efficiency_comparison.png)

Full methodology, per-language breakdown, ablations, and benchmark artifacts: [docs/benchmark-report.md](docs/benchmark-report.md).

## File Coverage

SIFS indexes code files by default, skipping generated files, dependency directories, and caches. It uses the `ignore` crate, so `.gitignore` files, Git excludes, global ignores, and hidden files behave exactly like familiar developer search tools.

Recognized extensions: Python, JavaScript, TypeScript, Go, Rust, Java, Kotlin, Ruby, PHP, C, C++, C#, Swift, Scala, Elixir, Dart, Lua, SQL, Bash, Zig, Haskell, Markdown, YAML, TOML, JSON.

Text-like documents (Markdown, YAML, TOML, JSON) are available through library options.

## Documentation

- [CLI usage](docs/cli.md) — every command and flag
- [Rust library](docs/library.md) — `SifsIndex`, search modes, filters, indexing options
- [MCP server](docs/mcp.md) — stdio protocol and tool schemas
- [Agent-native scorecard](docs/agent-native-scorecard.md) — agent-facing contract and readiness evidence
- [Benchmarking](docs/benchmarks.md) — quality, latency, embedding, and smoke benchmarks
- [Architecture](docs/architecture.md) — file selection, chunking, embedding, sparse search, dense search, hybrid ranking

## License

MIT
