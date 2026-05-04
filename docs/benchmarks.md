# Benchmarking

SIFS includes benchmark utilities for search quality, indexing speed, query
latency, and embedding parity checks. Use these tools when changing file
walking, chunking, model loading, sparse search, dense search, or reranking.

## Quality and latency benchmark

The `sifs-benchmark` binary runs annotated search tasks over one or more pinned
repositories. It reports NDCG quality metrics, query latency percentiles, index
time, indexed files, chunks, and category-level scores.

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
- `--top-k <count>` controls search depth for each task.
- `--latency-runs <count>` controls repeated query timing per task.
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
      "p50_ms": 0.28,
      "p90_ms": 0.41,
      "index_ms": 194.4,
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
    "avg_p50_ms": 0.28
  }
}
```

Use `ndcg10` to compare ranking quality and `p50_ms` or `p90_ms` to compare
query latency. Use `index_ms`, `peak_rss_mb`, `files`, and `chunks` when
investigating indexing, memory, file selection, or chunking changes.

## Local smoke benchmark

The `examples/bench.rs` example measures one path, one query, and repeated
query latency. It is useful for quick checks outside the annotated benchmark
suite.

```bash
cargo build --release --example bench
target/release/examples/bench /path/to/project "authentication flow" 100
```

The example prints index time, query latency percentiles, indexed files, and
chunk count. It also prints peak resident memory on Unix platforms.

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

These measurements were collected on May 4, 2026, on this development machine.
They are useful as a reference point, but they aren't a hardware-independent
performance contract.

Full Semble corpus comparison:

- SIFS full corpus: `repos=63`, `tasks=1251`, `NDCG@10=0.8444183076002316`,
  and `p50=0.0016651087130295766ms`.
- The detailed report, graphs, baseline comparison table, and per-language
  breakdown are in [Benchmark Report](benchmark-report.md).
- The raw SIFS payload is in
  [benchmarks/results/sifs-full.json](../benchmarks/results/sifs-full.json).

React large-repository smoke benchmark:

- Shallow clone of `facebook/react`: `index_ms=8289.240`,
  `query_p50_ms=0.002`, `query_p90_ms=0.003`, `peak_rss_mb=362.8`,
  `files=4373`, and `chunks=21117`.
- Captured output is in
  [benchmarks/results/react-smoke.txt](../benchmarks/results/react-smoke.txt).

FastAPI annotated benchmark:

- SIFS warm cache: `NDCG@5=0.7742068779742652`,
  `NDCG@10=0.8297832939936765`, `p50=0.00125ms`,
  `p90=0.004083ms`, `index=89.226292ms`, `peak_rss=175.265625MB`,
  `files=46`, and `chunks=603`.
- Python reference: `NDCG@10=0.786`, `p50=0.76ms`, `index=260ms`, and
  `chunks=597`.

Large local application smoke benchmark:

- SIFS cold build after the optimization pass: `index_ms=10148.225`,
  `query_p50_ms=0.001`, `query_p90_ms=0.001`, `peak_rss_mb=866.5`,
  `files=4435`, and `chunks=51350`.
- SIFS warm cache after the optimization pass: `index_ms=165.326`,
  `query_p50_ms=0.001`, `query_p90_ms=0.001`, `peak_rss_mb=507.8`,
  `files=4435`, and `chunks=51350`.
- Python reference: `index_ms=25520.934`, `query_p50_ms=6.702`,
  `query_p90_ms=7.292`, `files=4435`, and `chunks=50667`.

The earlier SIFS baseline for this large local application was
`index_ms=15904.490`, `query_p50_ms=2.218`, and `query_p90_ms=2.491`.
Before batched embedding, local peak RSS measurements were around
`2465-2775MB` for the same app. Before query caching, repeated-query p90 was
around `1.783-2.070ms` after the first optimization pass.

## Interpreting deltas

Small chunk-count differences can be valid when chunk boundaries differ while
file coverage stays the same. Large file-count differences usually point to
file walker behavior, root `.gitignore` handling, default ignored directories,
or document-file inclusion.

When a delta appears, compare these values in order:

1. Compare indexed file counts.
2. Compare the sorted indexed file lists.
3. Compare chunk counts per file.
4. Compare query quality and latency after file coverage matches.

## Next steps

Read [Architecture](architecture.md) for the pipeline that benchmarks exercise,
or read [Rust library usage](library.md) to build custom measurement harnesses.
