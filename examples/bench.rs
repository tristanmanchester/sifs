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

    let mut times = Vec::with_capacity(runs);
    let options = SearchOptions::new(10).with_mode(SearchMode::Hybrid);
    std::hint::black_box(index.search_with(&query, &options));
    for _ in 0..runs {
        let start = Instant::now();
        let results = index.search_with(&query, &options);
        std::hint::black_box(results);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = times[times.len() / 2];
    let p90 = times[(times.len() * 9 / 10).min(times.len() - 1)];
    let stats = index.stats();
    println!(
        "index_ms={index_ms:.3} query_p50_ms={p50:.3} query_p90_ms={p90:.3} peak_rss_mb={:.1} files={} chunks={}",
        peak_rss_mb(),
        stats.indexed_files,
        index.chunks.len()
    );
    Ok(())
}
