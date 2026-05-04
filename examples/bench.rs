use sifs::SifsIndex;
use sifs::types::SearchMode;
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
    std::hint::black_box(index.search(&query, 10, SearchMode::Hybrid, None, None, None));
    for _ in 0..runs {
        let start = Instant::now();
        let results = index.search(&query, 10, SearchMode::Hybrid, None, None, None);
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

#[cfg(target_os = "macos")]
fn peak_rss_mb() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if rc == 0 {
        let usage = unsafe { usage.assume_init() };
        usage.ru_maxrss as f64 / (1024.0 * 1024.0)
    } else {
        0.0
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn peak_rss_mb() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if rc == 0 {
        let usage = unsafe { usage.assume_init() };
        usage.ru_maxrss as f64 / 1024.0
    } else {
        0.0
    }
}

#[cfg(not(unix))]
fn peak_rss_mb() -> f64 {
    0.0
}
