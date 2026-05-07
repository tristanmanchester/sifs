# Benchmarking

SIFS includes benchmark utilities for search quality, indexing speed, query
latency, and embedding parity checks. Use them when changing file walking,
chunking, model loading, sparse search, dense search, or reranking.

The `sifs-benchmark` and `sifs-embed` binaries are diagnostics. Build them with
the explicit `diagnostics` feature.

```bash
cargo build --release --features diagnostics --bins
```

## Quality and latency benchmark

The `sifs-benchmark` binary runs annotated search tasks over one or more pinned
repositories. It reports NDCG quality metrics, query latency percentiles, index
time, indexed files, chunks, and category scores.

```bash
target/release/sifs-benchmark \
  --benchmarks-dir /path/to/benchmarks \
  --bench-root /tmp/sifs-bench \
  --sync \
  --output /tmp/sifs-results.json
```

The benchmark directory must contain `repos.json` and an `annotations`
directory. Use `--sync` to clone, fetch, and check out the revisions named in
`repos.json`.

## Benchmark directory format

The `repos.json` file stores repository metadata. Each entry names a repository,
language, Git URL, pinned revision, and optional subdirectory to index.

```json
[
  {
    "name": "fastapi",
    "language": "python",
    "url": "https://github.com/fastapi/fastapi",
    "revision": "0123456789abcdef",
    "benchmark_root": null
  }
]
```

Annotation files live under `annotations/*.json`. The file stem is used as the
default repository name unless a task includes its own `repo` field.

```json
[
  {
    "query": "where are route dependencies resolved",
    "category": "architecture",
    "relevant": [
      {
        "path": "fastapi/dependencies/utils.py",
        "start_line": 1,
        "end_line": 120
      }
    ]
  }
]
```

Targets can be a path string or an object with `path`, `start_line`, and
`end_line`. `secondary` targets are treated as additional relevant targets.

## Benchmark filters

Use filters to run a smaller benchmark set while iterating.

```bash
target/release/sifs-benchmark \
  --benchmarks-dir /path/to/benchmarks \
  --bench-root /tmp/sifs-bench \
  --repo fastapi \
  --top-k 10 \
  --latency-runs 20
```

Available filters and controls are:

- `--repo <name>` can be repeated to include specific repositories.
- `--language <language>` can be repeated to include specific languages.
- `--mode <hybrid|semantic|bm25>` selects the search mode for the run.
- `--top-k <count>` controls search depth for each task.
- `--latency-runs <count>` controls repeated query timing per task.
- `--include-tasks` includes per-task ranks and result metadata for analysis
  plots such as context-efficiency curves.
- `--output <path>` writes the JSON payload to a file.

## Output payload

The benchmark prints or writes a JSON payload with a `method`, per-repository
results, and a weighted summary.

```json
{
  "method": "sifs-hybrid",
  "results": [
    {
      "repo": "fastapi",
      "language": "python",
      "chunks": 603,
      "files": 46,
      "tasks": 10,
      "ndcg5": 0.72,
      "ndcg10": 0.79,
      "cold_index_ms": 194.4,
      "warm_uncached_query_ms": 0.28,
      "warm_uncached_query_p90_ms": 0.41,
      "warm_cached_repeat_query_ms": 0.0017,
      "warm_cached_repeat_query_p90_ms": 0.0021,
      "peak_rss_mb": 208.1,
      "by_category": {
        "architecture": 0.81
      }
    }
  ],
  "summary": {
    "repos": 1,
    "tasks": 10,
    "avg_ndcg10": 0.79,
    "avg_cold_index_ms": 194.4,
    "avg_warm_uncached_query_ms": 0.28,
    "avg_warm_cached_repeat_query_ms": 0.0017
  }
}
```

Use `ndcg10` to compare ranking quality. Use `warm_uncached_query_ms` for
normal searches after an index exists, and `warm_cached_repeat_query_ms` only
for identical repeated queries inside the same process. Use `cold_index_ms`,
`peak_rss_mb`, `files`, and `chunks` when investigating indexing, memory, file
selection, or chunking changes.

## Local smoke benchmark

The `examples/bench.rs` example measures one path, one query, and repeated
query latency. Use it for quick checks outside the annotated benchmark suite.

```bash
cargo build --release --example bench
target/release/examples/bench /path/to/project "authentication flow" 100
```

The example prints cold index time, warm uncached query percentiles, cached
repeat query percentiles, indexed files, and chunk count. It also prints peak
resident memory on Unix platforms.

## Embedding helper

The `sifs-embed` binary encodes one text string with the SIFS embedding model
and prints the vector as JSON. Use it for model-loader checks and parity tests.

```bash
target/release/sifs-embed "parse oauth callback"
target/release/sifs-embed "parse oauth callback" --model /path/to/model
target/release/sifs-embed "parse oauth callback" --no-download
```

The `--model` value can point to a local Model2Vec model path. Without it, SIFS
uses `SIFS_MODEL` or the default code-search model. Use `--no-download` or
`--offline` to require an already-local model.

## Recent local measurements

These measurements were collected on May 7, 2026, on this development machine.
They are useful as a reference point, but they aren't a hardware-independent
performance contract.

Full annotated corpus comparison:

- SIFS full corpus: `repos=63`, `tasks=1251`, `NDCG@10=0.841774576215374`,
  `cold_index_ms=182.25507682014387`,
  `warm_uncached_query_ms=4.1817529208633095`, and
  `warm_cached_repeat_query_ms=0.005043049560351717`.
- The detailed report, graphs, baseline comparison table, and per-language
  breakdown are in [Benchmark Report](benchmark-report.md).
- The raw SIFS payload is in
  [benchmarks/results/sifs-full.json](../benchmarks/results/sifs-full.json).

React large-repository smoke benchmark:

- Shallow clone of `facebook/react`: `cold_index_ms=2137.435`,
  `warm_uncached_query_ms=2.053`, `warm_uncached_query_p90_ms=2.322`,
  `warm_cached_repeat_query_ms=0.001`,
  `warm_cached_repeat_query_p90_ms=0.001`, `peak_rss_mb=461.9`,
  `files=4370`, and `chunks=21096`.
- Captured output is in
  [benchmarks/results/react-smoke.txt](../benchmarks/results/react-smoke.txt).

## Interpreting deltas

Small chunk-count differences can be valid when chunk boundaries differ while
file coverage stays the same. Large file-count differences usually point to
file walker behavior, ignore-file handling, default ignored directories,
skipped-file warnings, or document-file inclusion.

When a delta appears, compare these values in order:

1. Compare indexed file counts.
2. Compare the sorted indexed file lists.
3. Compare chunk counts per file.
4. Compare query quality and latency after file coverage matches.
