use sifs::{SearchMode, SearchOptions, SifsIndex, metrics::peak_rss_mb};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| ".".to_owned());
    let query = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "hybrid search ranking".to_owned());
    let runs = std::env::args()
        .nth(3)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(100);

    let start = Instant::now();
    let index = SifsIndex::from_path(&path)?;
    let index_ms = start.elapsed().as_secs_f64() * 1000.0;

    let mut uncached_times = Vec::with_capacity(runs);
    let mut cached_times = Vec::with_capacity(runs);
    let uncached = SearchOptions::new(10)
        .with_mode(SearchMode::Hybrid)
        .with_cache(false);
    let cached = SearchOptions::new(10)
        .with_mode(SearchMode::Hybrid)
        .with_cache(true);
    std::hint::black_box(index.search_with(&query, &uncached)?);
    for _ in 0..runs {
        let start = Instant::now();
        let results = index.search_with(&query, &uncached)?;
        std::hint::black_box(results);
        uncached_times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    std::hint::black_box(index.search_with(&query, &cached)?);
    for _ in 0..runs {
        let start = Instant::now();
        let results = index.search_with(&query, &cached)?;
        std::hint::black_box(results);
        cached_times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    uncached_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    cached_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let uncached_p50 = uncached_times[uncached_times.len() / 2];
    let uncached_p90 =
        uncached_times[(uncached_times.len() * 9 / 10).min(uncached_times.len() - 1)];
    let cached_p50 = cached_times[cached_times.len() / 2];
    let cached_p90 = cached_times[(cached_times.len() * 9 / 10).min(cached_times.len() - 1)];
    let stats = index.stats();
    println!(
        "cold_index_ms={index_ms:.3} warm_uncached_query_ms={uncached_p50:.3} warm_uncached_query_p90_ms={uncached_p90:.3} warm_cached_repeat_query_ms={cached_p50:.3} warm_cached_repeat_query_p90_ms={cached_p90:.3} peak_rss_mb={:.1} files={} chunks={}",
        peak_rss_mb(),
        stats.indexed_files,
        index.chunks.len()
    );
    Ok(())
}
