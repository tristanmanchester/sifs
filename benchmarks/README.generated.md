# SIFS Benchmark Report

Source SIFS result: `benchmarks/results/sifs-full.json`

## Main Results

| Method | NDCG@10 | Cold index | Warm uncached query | Cached repeat query |
|---|---:|---:|---:|---:|
| CodeRankEmbed Hybrid | 0.8617 | 57.3 s | 16.9 ms | n/a |
| Semble | 0.8544 | 439.4 ms | 1.3 ms | n/a |
| **SIFS** | 0.8029 | 183.5 ms | 4.0 ms | 0.0029 ms |
| CodeRankEmbed | 0.7648 | 57.3 s | 13.3 ms | n/a |
| ColGREP | 0.6925 | 3.9 s | 979.3 ms | n/a |
| grepai | 0.5606 | 35.0 s | 47.7 ms | n/a |
| probe | 0.3872 | 0.0000 ms | 207.1 ms | n/a |
| ripgrep | 0.1257 | 0.0000 ms | 8.8 ms | n/a |

## Notes

- SIFS results were produced by the Rust `sifs-benchmark` binary against the annotated pinned-repository corpus.
- Baseline methods use existing comparison result JSON files from the adjacent Python tool checkout.
- Warm uncached query latency bypasses the in-process SIFS query-result cache. Cached repeat query latency measures identical repeated queries after one warm-up.
- Some baseline files only expose precomputed summary timing fields; the report preserves those values.

## Generated Figures

- `assets/images/speed_vs_quality_combined.png`
- `assets/images/speed_vs_quality_cold.png`
- `assets/images/speed_vs_quality_warm.png`
- `assets/images/quality_vs_warm_latency.png`
- `assets/images/context_efficiency_comparison.png`
- `assets/images/query_type_quality_by_mode.png`
- `assets/images/sifs_by_language.png`
- `assets/images/sifs_by_category.png`
