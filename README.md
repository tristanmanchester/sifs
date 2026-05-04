<p align="center">
  <img alt="SIFS Is Fast Search" src="assets/logo/sifs-logo.png" width="220">
</p>

# SIFS

SIFS ("**S**IFS **I**s **F**ast **S**earch") is a Rust code search tool and library for
agents, editor integrations, and local developer workflows. It indexes a
repository, splits files into useful chunks, embeds those chunks with a
Model2Vec-compatible encoder, and runs hybrid search over semantic and BM25
rankings.

On the 63-repository Semble benchmark corpus, SIFS reached `NDCG@10=0.8444`
with `93.0ms` average indexing time and `0.0017ms` repeated-query p50 latency.
That is `0.0100` NDCG@10 behind Semble and `0.0173` behind CodeRankEmbed
Hybrid, with a much faster warm query path in this benchmark.

## What SIFS does

SIFS turns a local directory or Git repository into a searchable code index. You
can use it as a command-line tool, as a Rust crate, or as a Model Context
Protocol server for agent clients.

- Search code with natural-language or symbol-heavy queries.
- Find chunks related to a specific file and line.
- Index local directories or shallow-cloned Git repositories.
- Use hybrid, semantic-only, or BM25-only ranking.
- Run BM25 search fully offline without loading or downloading a model.
- Use explicit sparse-only indexes or model-free hashing for local smoke tests.
- Run quality and latency benchmarks over annotated repositories.

## Install SIFS

SIFS is intended to ship as the `sifs` command. Package-manager installs should
install that binary by default.

```bash
cargo install --locked sifs
```

Homebrew installs are published through
[`tristanmanchester/homebrew-tap`](https://github.com/tristanmanchester/homebrew-tap).
The draft formula in [packaging/homebrew/sifs.rb](packaging/homebrew/sifs.rb)
is the source-repo template for the tap formula.

## Build SIFS

SIFS builds with Cargo. The default release build gives you the public `sifs`
binary under `target/release/`.

```bash
cargo build --release
```

The `sifs-benchmark` and `sifs-embed` binaries are supported diagnostics for
benchmarking and embedding-model checks. Build them explicitly with the
`diagnostics` feature.

```bash
cargo build --release --features diagnostics --bins
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
target/release/sifs search "parse JWT claims" /path/to/project --mode bm25 --offline -k 10
target/release/sifs search "auth flow" /path/to/project --mode semantic --encoder hashing
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

Start with the page that matches how you plan to use SIFS.

- [Command-line usage](docs/cli.md) covers `search`, `find-related`, `init`,
  and MCP server startup.
- [Rust library usage](docs/library.md) covers `SifsIndex`, search modes,
  filters, and indexing options.
- [MCP server usage](docs/mcp.md) covers the stdio protocol surface and tool
  schemas.
- [Agent-native scorecard](docs/agent-native-scorecard.md) defines the
  agent-facing contract and readiness evidence.
- [Benchmarking](docs/benchmarks.md) covers quality, latency, embedding, and
  local smoke benchmarks.
- [Architecture](docs/architecture.md) explains file selection, chunking,
  embedding, sparse search, dense search, and hybrid ranking.

## Search model

SIFS uses `minishlab/potion-code-16M` by default through a local Model2Vec
loader. The loader reads the model tensors and tokenizer files directly, so the
query path stays inside the Rust process after the model is available locally.
BM25 mode does not use the model at all, so it is safe for network-free package
manager smoke tests and first-run checks.

Use `sifs model pull` or `sifs model fetch` to prefetch the default model, and
`sifs doctor` to check whether semantic search is ready for offline use.

Hybrid search combines semantic and BM25 rankings. It over-fetches candidates,
normalizes each ranking with reciprocal rank fusion, applies query-aware boosts,
and reranks the top results. Symbol-like queries lean more heavily on BM25,
while natural-language queries keep more semantic weight.

## Benchmarks

The current full-corpus benchmark uses the Semble benchmark suite: 63 pinned
open-source repositories, 19 languages, and 1,251 annotated search tasks. In
this local run, SIFS reached `NDCG@10=0.8444` with `p50=0.0017ms`
repeated-query latency after indexing.

SIFS is third on raw NDCG@10 in this comparison, behind CodeRankEmbed Hybrid and
Semble. It is `0.0100` NDCG@10 behind Semble and `0.0173` behind CodeRankEmbed
Hybrid, while reporting much lower warm-query latency and lower average index
time than the embedding-heavy baselines.

| Method | NDCG@10 | Index time | Query p50 |
|---|---:|---:|---:|
| CodeRankEmbed Hybrid | 0.8617 | 57.3 s | 16.9 ms |
| semble | 0.8544 | 439.4 ms | 1.3 ms |
| **SIFS** | **0.8444** | **93.0 ms** | **0.0017 ms** |
| CodeRankEmbed | 0.7648 | 57.3 s | 13.3 ms |
| ColGREP | 0.6925 | 3.9 s | 979.3 ms |
| grepai | 0.5606 | 35.0 s | 47.7 ms |
| probe | 0.3872 | 0.0000 ms | 207.1 ms |
| ripgrep | 0.1257 | 0.0000 ms | 8.8 ms |

![SIFS speed and quality compared with code-search baselines](assets/images/speed_vs_quality_combined.png)

SIFS is strongest on symbol-heavy queries while still performing well on
semantic and architecture questions.

| Query category | NDCG@10 |
|---|---:|
| symbol | 0.9566 |
| semantic | 0.8262 |
| architecture | 0.8070 |

![SIFS quality by query category](assets/images/sifs_by_category.png)

The benchmark artifacts live in [benchmarks/results](benchmarks/results), and
the full methodology, per-language breakdown, additional figures, and React
large-repository smoke result are in [docs/benchmark-report.md](docs/benchmark-report.md).

## File coverage

By default, SIFS indexes code files and skips common generated, dependency, and
cache directories. It uses the `ignore` crate, so nested `.gitignore` files,
Git excludes, global Git ignores, and hidden files behave like familiar
developer search tools. Text-like documents such as Markdown, YAML, TOML, and
JSON are available through the library options.

The file walker currently recognizes Python, JavaScript, TypeScript, Go, Rust,
Java, Kotlin, Ruby, PHP, C, C++, C#, Swift, Scala, Elixir, Dart, Lua, SQL,
Bash, Zig, Haskell, Markdown, YAML, TOML, and JSON extensions.
