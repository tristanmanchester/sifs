# SIFS Benchmark Report

Source SIFS result: `benchmarks/results/sifs-full.json`

## Main Results

| Method | NDCG@10 | Index time | Query p50 |
|---|---:|---:|---:|
| CodeRankEmbed Hybrid | 0.8617 | 57.3 s | 16.9 ms |
| semble | 0.8544 | 439.4 ms | 1.3 ms |
| **SIFS** | 0.8444 | 93.0 ms | 0.0017 ms |
| CodeRankEmbed | 0.7648 | 57.3 s | 13.3 ms |
| ColGREP | 0.6925 | 3.9 s | 979.3 ms |
| grepai | 0.5606 | 35.0 s | 47.7 ms |
| probe | 0.3872 | 0.0000 ms | 207.1 ms |
| ripgrep | 0.1257 | 0.0000 ms | 8.8 ms |

## Notes

- SIFS results were produced by the Rust `sifs-benchmark` binary against the Semble benchmark annotations and pinned repositories.
- Other methods use the existing Semble benchmark result JSON files in `semble/benchmarks/results`.
- Cold latency is index time plus first query latency. Warm latency is query p50 with an existing index.
- Existing Semble baseline files include some methods with precomputed summary-only timing fields; the report preserves those values.

## Generated Figures

- `assets/images/speed_vs_quality_combined.png`
- `assets/images/speed_vs_quality_cold.png`
- `assets/images/speed_vs_quality_warm.png`
- `assets/images/sifs_by_language.png`
- `assets/images/sifs_by_category.png`
